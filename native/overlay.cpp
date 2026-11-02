// MLauncher optional OpenGL overlay. Only loaded into a child launched with explicit consent.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winhttp.h>
#include <GL/gl.h>
#include <atomic>
#include <condition_variable>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <algorithm>
#include <cmath>
#include "MinHook.h"
#include "imgui.h"
#include "backends/imgui_impl_win32.h"
#include "backends/imgui_impl_opengl3.h"
#include "json.hpp"

extern IMGUI_IMPL_API LRESULT ImGui_ImplWin32_WndProcHandler(HWND,UINT,WPARAM,LPARAM);
using json=nlohmann::json;
using SwapFn=BOOL(WINAPI*)(HDC);
using ClipFn=BOOL(WINAPI*)(const RECT*);
using PositionFn=BOOL(WINAPI*)(int,int);
static SwapFn originalSwap=nullptr;
static ClipFn originalClip=nullptr;
static PositionFn originalPosition=nullptr;
static HWND target=nullptr;
static HGLRC targetContext=nullptr;
static WNDPROC originalWndProc=nullptr;
static ImGuiContext* context=nullptr;
static std::atomic<bool> visible{false},busy{false},rendererReady{false};
static std::mutex dataMutex,pollMutex;
static std::condition_variable pollWake;
static json data={{"entries",json::array()},{"logs",json::array()},{"name","Minecraft"},{"status",""}};
static std::atomic<unsigned> revision{1};
static std::wstring token;
static INTERNET_PORT port=0;
static RECT previousClip{};
static int cursorChanges=0;
static bool rendererFailed=false;
static bool testMode=false;
static const UINT WM_MLAUNCHER_TOGGLE=WM_APP+1;
using BindFramebufferFn=void(APIENTRY*)(GLenum,GLuint);
static BindFramebufferFn bindFramebuffer=nullptr;
static ImVec4 accentColor(0.47f,0.73f,0.67f,1.f);
static bool lightStyle=false;
static int effectStyle=2; // 0 none, 1 waves, 2 bubbles, 3 aurora

struct HttpHandle {
    HINTERNET handle=nullptr;
    explicit HttpHandle(HINTERNET h):handle(h){}
    ~HttpHandle(){if(handle)WinHttpCloseHandle(handle);}
    operator HINTERNET() const{return handle;}
};
static json rpc(const wchar_t* path,const json* body=nullptr) {
    HttpHandle session(WinHttpOpen(L"MLauncher Overlay/0.3.4",WINHTTP_ACCESS_TYPE_NO_PROXY,nullptr,nullptr,0));
    if(!session.handle)throw std::runtime_error("WinHTTP initialization failed");
    WinHttpSetTimeouts(session,2000,2000,3000,180000);
    HttpHandle connection(WinHttpConnect(session,L"127.0.0.1",port,0));
    HttpHandle request(WinHttpOpenRequest(connection,body?L"POST":L"GET",path,nullptr,WINHTTP_NO_REFERER,WINHTTP_DEFAULT_ACCEPT_TYPES,0));
    if(!request.handle)throw std::runtime_error("Launcher connection unavailable");
    const std::wstring headers=L"X-MLauncher-Token: "+token+L"\r\nContent-Type: application/json\r\n";
    std::string payload=body?body->dump():"";
    if(!WinHttpSendRequest(request,headers.c_str(),static_cast<DWORD>(-1),payload.empty()?nullptr:payload.data(),static_cast<DWORD>(payload.size()),static_cast<DWORD>(payload.size()),0)||!WinHttpReceiveResponse(request,nullptr))
        throw std::runtime_error("Нет связи с лаунчером. Он должен оставаться открытым.");
    std::string result;char buffer[8192];DWORD read=0;
    while(WinHttpReadData(request,buffer,sizeof(buffer),&read)&&read){
        if(result.size()+read>1024*1024)throw std::runtime_error("Launcher response too large");
        result.append(buffer,read);
    }
    return json::parse(result);
}
static void nativeStatus(const std::string& message) {
    try{json body={{"message",message}};rpc(L"/ready",&body);}catch(...){}
}
static void applyStyle() {
    ImGuiStyle& style=ImGui::GetStyle();
    style.WindowRounding=10.f;style.FrameRounding=5.f;style.TabRounding=4.f;
    style.WindowBorderSize=1.f;style.FrameBorderSize=0.f;style.WindowPadding=ImVec2(14,12);
    ImVec4* colors=style.Colors;
    const ImVec4 base=lightStyle?ImVec4(.93f,.94f,.96f,.97f):ImVec4(.18f,.24f,.26f,.97f);
    const ImVec4 frame=lightStyle?ImVec4(.85f,.88f,.91f,1):ImVec4(.27f,.31f,.32f,1);
    const ImVec4 text=lightStyle?ImVec4(.12f,.14f,.18f,1):ImVec4(.84f,.86f,.88f,1);
    colors[ImGuiCol_WindowBg]=base;colors[ImGuiCol_ChildBg]=ImVec4(base.x,base.y,base.z,.55f);
    colors[ImGuiCol_PopupBg]=base;colors[ImGuiCol_Text]=text;colors[ImGuiCol_TextDisabled]=ImVec4(text.x,text.y,text.z,.55f);
    colors[ImGuiCol_FrameBg]=frame;colors[ImGuiCol_FrameBgHovered]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.30f);
    colors[ImGuiCol_FrameBgActive]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.45f);
    colors[ImGuiCol_Button]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.42f);
    colors[ImGuiCol_ButtonHovered]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.62f);
    colors[ImGuiCol_ButtonActive]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.80f);
    colors[ImGuiCol_Header]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.45f);
    colors[ImGuiCol_HeaderHovered]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.62f);
    colors[ImGuiCol_Tab]=frame;colors[ImGuiCol_TabHovered]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.55f);
    colors[ImGuiCol_TabSelected]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.60f);
    colors[ImGuiCol_CheckMark]=accentColor;colors[ImGuiCol_SliderGrab]=accentColor;
    colors[ImGuiCol_Border]=ImVec4(accentColor.x,accentColor.y,accentColor.z,.55f);
    colors[ImGuiCol_TitleBg]=ImVec4(base.x,base.y,base.z,1);colors[ImGuiCol_TitleBgActive]=ImVec4(base.x,base.y,base.z,1);
}
static void drawEffectBackground(const ImVec2 screen) {
    ImDrawList* draw=ImGui::GetBackgroundDrawList();
    const ImU32 accent=ImGui::ColorConvertFloat4ToU32(ImVec4(accentColor.x,accentColor.y,accentColor.z,.20f));
    const float t=static_cast<float>(ImGui::GetTime());
    if(effectStyle==1){
        for(int band=0;band<7;++band){
            std::vector<ImVec2> points;points.reserve(80);
            for(int i=0;i<80;++i){const float x=i/79.f;const float y=.22f+band*.09f+sinf(x*6.2f+t*.5f+band*.7f)*.055f;points.push_back(ImVec2(x*screen.x,y*screen.y));}
            draw->AddPolyline(points.data(),static_cast<int>(points.size()),accent,ImDrawFlags_None,2.f);
        }
    }else if(effectStyle==2){
        for(int i=0;i<34;++i){const float n=static_cast<float>(i);const float x=fmodf(n*.618f+t*.018f,1.f);const float y=fmodf(1.2f-n*.37f-t*(.012f+fmodf(n,4.f)*.003f),1.3f);draw->AddCircle(ImVec2(x*screen.x,y*screen.y),10.f+fmodf(n*17.f,40.f),accent,32,1.5f);}
    }else if(effectStyle==3){
        for(int band=0;band<22;++band){std::vector<ImVec2> points;points.reserve(80);const float b=static_cast<float>(band);
            for(int i=0;i<80;++i){const float x=i/79.f;const float y=.30f+b*.008f+sinf(x*7.f+t*.25f)*.11f+sinf(x*13.f-t*.16f)*.045f;points.push_back(ImVec2(x*screen.x,y*screen.y));}
            const ImU32 color=ImGui::ColorConvertFloat4ToU32(ImVec4(.22f,.78f,.66f,.025f));
            draw->AddPolyline(points.data(),static_cast<int>(points.size()),color,ImDrawFlags_None,14.f);
        }
    }
}
static void setVisible(bool show) {
    if(visible.load()==show)return;
    if(show){
        // Release keys delivered before the toggle, avoiding a stuck walking/key state in GLFW.
        if(originalWndProc&&target){
            for(UINT key=8;key<256;++key)if(GetAsyncKeyState(static_cast<int>(key))&0x8000)
                CallWindowProcW(originalWndProc,target,WM_KEYUP,key,(1u<<30)|(1u<<31));
            CallWindowProcW(originalWndProc,target,WM_LBUTTONUP,0,0);
            CallWindowProcW(originalWndProc,target,WM_RBUTTONUP,0,0);
        }
        GetClipCursor(&previousClip);ReleaseCapture();
        visible.store(true);
        if(originalClip)originalClip(nullptr);
        cursorChanges=0;
        do{++cursorChanges;}while(ShowCursor(TRUE)<0&&cursorChanges<32);
        SetCursor(LoadCursorW(nullptr,IDC_ARROW));
        pollWake.notify_one();
    }else{
        visible.store(false);
        while(cursorChanges>0){ShowCursor(FALSE);--cursorChanges;}
        if(originalClip&&GetForegroundWindow()==target)originalClip(&previousClip);
    }
}
static BOOL WINAPI clipHook(const RECT* rect) {
    if(visible.load()&&GetForegroundWindow()==target)return originalClip(nullptr);
    return originalClip(rect);
}
static BOOL WINAPI positionHook(int x,int y) {
    if(visible.load()&&GetForegroundWindow()==target)return TRUE;
    return originalPosition(x,y);
}
static LRESULT CALLBACK windowProc(HWND window,UINT message,WPARAM w,LPARAM l) {
    if(message==WM_MLAUNCHER_TOGGLE){setVisible(!visible.load());return 0;}
    // Real input uses the polling hotkey; only the isolated synthetic smoke test uses messages.
    if(testMode&&message==WM_KEYDOWN&&w==VK_TAB&&(GetKeyState(VK_SHIFT)&0x8000)&&!(l&(1u<<30))){setVisible(!visible.load());return 0;}
    if(message==WM_KILLFOCUS&&visible.load())setVisible(false);
    if(visible.load()&&context){
        ImGui::SetCurrentContext(context);
        if(message==WM_KEYDOWN&&w==VK_ESCAPE){setVisible(false);return 0;}
        ImGui_ImplWin32_WndProcHandler(window,message,w,l);
        switch(message){
            case WM_INPUT:return DefWindowProcW(window,message,w,l);
            case WM_KEYDOWN:case WM_KEYUP:case WM_CHAR:case WM_UNICHAR:
            case WM_MOUSEMOVE:case WM_MOUSEWHEEL:case WM_MOUSEHWHEEL:
            case WM_LBUTTONDOWN:case WM_LBUTTONUP:case WM_LBUTTONDBLCLK:
            case WM_RBUTTONDOWN:case WM_RBUTTONUP:case WM_RBUTTONDBLCLK:
            case WM_MBUTTONDOWN:case WM_MBUTTONUP:case WM_XBUTTONDOWN:case WM_XBUTTONUP:return 0;
            case WM_SETCURSOR:SetCursor(LoadCursorW(nullptr,IDC_ARROW));return TRUE;
        }
    }
    return CallWindowProcW(originalWndProc,window,message,w,l);
}
static void runJob(const std::wstring& path,json body) {
    if(busy.exchange(true))return;
    {std::lock_guard<std::mutex> lock(dataMutex);data["status"]="Загрузка…";++revision;}
    std::thread([path,body]{
        try{
            json result=rpc(path.c_str(),&body);
            std::lock_guard<std::mutex> lock(dataMutex);
            if(result.contains("error"))data["status"]=result["error"];
            else if(path==L"/resources"){
                data["provider"]=body["provider"];
                data["entries"]=result.value("entries",json::array());
                data["name"]=result.value("name",std::string("Minecraft"));
                data["minecraft"]=result.value("minecraft",std::string());
                data["status"]="";
            }else{
                data["status"]=result.value("message",std::string("Готово"));
                for(auto& entry:data["entries"])if(entry["id"]==body["id"])entry["installed"]=true;
            }
            ++revision;
        }catch(const std::exception& error){std::lock_guard<std::mutex> lock(dataMutex);data["status"]=error.what();++revision;}
        busy.store(false);
    }).detach();
}
static void pollLogs() {
    for(;;){
        {std::unique_lock<std::mutex> lock(pollMutex);pollWake.wait(lock,[]{return visible.load();});}
        try{
            const json result=rpc(L"/logs");
            std::lock_guard<std::mutex> lock(dataMutex);
            if(result.contains("logs")){data["logs"]=result["logs"];data["name"]=result.value("name",std::string("Minecraft"));++revision;}
        }catch(const std::exception& error){std::lock_guard<std::mutex> lock(dataMutex);data["status"]=error.what();++revision;}
        std::this_thread::sleep_for(std::chrono::milliseconds(750));
    }
}
static void drawOverlay() {
    static json snapshot;static unsigned lastRevision=0;
    if(lastRevision!=revision.load()){
        std::lock_guard<std::mutex> lock(dataMutex);snapshot=data;lastRevision=revision.load();
    }
    const ImVec2 screen=ImGui::GetIO().DisplaySize;
    drawEffectBackground(screen);
    ImGui::GetBackgroundDrawList()->AddRectFilled(ImVec2(0,0),screen,lightStyle?IM_COL32(220,226,232,70):IM_COL32(0,0,0,140));
    ImGui::SetNextWindowSize(ImVec2(std::min(920.f,screen.x*.90f),std::min(680.f,screen.y*.9f)),ImGuiCond_FirstUseEver);
    ImGui::SetNextWindowPos(ImVec2(screen.x*.5f,screen.y*.5f),ImGuiCond_FirstUseEver,ImVec2(.5f,.5f));
    bool open=true;
    if(ImGui::Begin("MLauncher · Shift+Tab / Esc",&open,ImGuiWindowFlags_NoCollapse)){
        ImGui::TextUnformatted(snapshot.value("name",std::string("Minecraft")).c_str());
        ImGui::TextDisabled("Ресурспаки и журнал этой сессии · без установки мода");
        if(ImGui::BeginTabBar("overlay-tabs")){
            if(ImGui::BeginTabItem("Ресурспаки")){
                static char query[257]="";static int provider=0,sort=0;
                ImGui::SetNextItemWidth(160);ImGui::Combo("##provider",&provider,"Modrinth\0CurseForge\0");ImGui::SameLine();
                ImGui::SetNextItemWidth(210);ImGui::Combo("##sort",&sort,"По скачиваниям\0Релевантность\0Обновлённые\0Новые\0");
                ImGui::SetNextItemWidth(-100);bool search=ImGui::InputText("##search",query,sizeof(query),ImGuiInputTextFlags_EnterReturnsTrue);ImGui::SameLine();
                ImGui::BeginDisabled(busy.load());search=ImGui::Button("Найти")||search;ImGui::EndDisabled();
                if(search&&!busy.load()){
                    static const char* sorts[]={"downloads","relevance","updated","newest"};
                    runJob(L"/resources",{{"query",query},{"provider",provider==0?"modrinth":"curseforge"},{"sort",sorts[sort]}});
                }
                ImGui::TextWrapped("%s",snapshot.value("status",std::string()).c_str());
                ImGui::TextDisabled("После скачивания включите ресурспак в настройках Minecraft.");
                ImGui::BeginChild("cards",ImVec2(0,0),true);
                for(const auto& entry:snapshot.value("entries",json::array())){
                    const std::string id=entry.value("id",std::string());ImGui::PushID(id.c_str());
                    ImGui::TextWrapped("%s",entry.value("name",std::string()).c_str());
                    ImGui::TextDisabled("%s",entry.value("author",std::string()).c_str());
                    ImGui::TextWrapped("%s",entry.value("description",std::string()).c_str());
                    const bool installed=entry.value("installed",false);ImGui::BeginDisabled(busy.load()||installed);
                    if(ImGui::Button(installed?"Установлено":"Скачать"))runJob(L"/download",{{"id",id},{"provider",snapshot.value("provider",std::string("modrinth"))}});
                    ImGui::EndDisabled();ImGui::Separator();ImGui::PopID();
                }
                ImGui::EndChild();ImGui::EndTabItem();
            }
            if(ImGui::BeginTabItem("Журнал")){
                static bool follow=true;ImGui::Checkbox("Прокручивать вниз",&follow);
                ImGui::BeginChild("logs",ImVec2(0,0),true,ImGuiWindowFlags_HorizontalScrollbar);
                const bool atBottom=ImGui::GetScrollY()>=ImGui::GetScrollMaxY()-2.f;
                for(const auto& line:snapshot.value("logs",json::array()))if(line.is_string())ImGui::TextUnformatted(line.get_ref<const std::string&>().c_str());
                if(follow&&atBottom)ImGui::SetScrollHereY(1.f);
                ImGui::EndChild();ImGui::EndTabItem();
            }
            if(ImGui::BeginTabItem("О проекте")){
                ImGui::TextWrapped("Экспериментальный нативный оверлей MLauncher для Windows x64 / OpenGL 3+. Vulkan / VulkanMod НЕ поддерживаются. Возможны конфликты с Discord/Steam и античитами. Отключение: настройки лаунчера и перезапуск игры.");
                ImGui::TextWrapped("Лаунчер должен оставаться открытым. Запросы выполняются в фоне через локальную связь с отдельным токеном этой сессии.");
                ImGui::EndTabItem();
            }
            ImGui::EndTabBar();
        }
    }
    const ImVec2 windowPosition=ImGui::GetWindowPos(),windowSize=ImGui::GetWindowSize();
    ImGui::End();
    const ImVec2 mouse=ImGui::GetIO().MousePos;
    const bool outside=mouse.x<windowPosition.x||mouse.y<windowPosition.y||mouse.x>windowPosition.x+windowSize.x||mouse.y>windowPosition.y+windowSize.y;
    if(!open||(ImGui::GetIO().MouseClicked[0]&&outside))setVisible(false);
}
static BOOL WINAPI swapHook(HDC dc) {
    const HWND window=WindowFromDC(dc);const HGLRC current=wglGetCurrentContext();
    // This DLL exists only inside our child Java process. Avoid depending on a window class:
    // launchers/mods may create wrappers, but the game's GL context is still the one to draw.
    if(rendererFailed||!window||!current)return originalSwap(dc);
    try{
        if(!context){
            const char* version=reinterpret_cast<const char*>(glGetString(GL_VERSION));
            if(!version||atoi(version)<3)return originalSwap(dc);
            target=window;targetContext=current;
            bindFramebuffer=reinterpret_cast<BindFramebufferFn>(wglGetProcAddress("glBindFramebuffer"));
            IMGUI_CHECKVERSION();context=ImGui::CreateContext();
            ImGui::GetIO().IniFilename=nullptr;ImGui::GetIO().LogFilename=nullptr;
            wchar_t windows[MAX_PATH]{};GetWindowsDirectoryW(windows,MAX_PATH);
            std::wstring font=std::wstring(windows)+L"\\Fonts\\segoeui.ttf";
            const int length=WideCharToMultiByte(CP_UTF8,0,font.c_str(),-1,nullptr,0,nullptr,nullptr);
            std::string fontPath(length,'\0');WideCharToMultiByte(CP_UTF8,0,font.c_str(),-1,fontPath.data(),length,nullptr,nullptr);
            ImGui::GetIO().Fonts->AddFontFromFileTTF(fontPath.c_str(),18.f,nullptr,ImGui::GetIO().Fonts->GetGlyphRangesCyrillic());
            ImGui::StyleColorsDark();applyStyle();
            if(!ImGui_ImplWin32_Init(window)||!ImGui_ImplOpenGL3_Init("#version 130")){
                rendererFailed=true;context=nullptr;return originalSwap(dc);
            }
            originalWndProc=reinterpret_cast<WNDPROC>(SetWindowLongPtrW(window,GWLP_WNDPROC,reinterpret_cast<LONG_PTR>(windowProc)));
            if(!originalWndProc){rendererFailed=true;context=nullptr;return originalSwap(dc);}
            rendererReady.store(true);
            std::thread(pollLogs).detach();
            // Polling works even if GLFW/raw input does not deliver WM_KEYDOWN to the hooked WndProc.
            std::thread([]{bool held=false;for(;;){Sleep(40);if(!target)continue;const bool down=(GetAsyncKeyState(VK_SHIFT)&0x8000)&&(GetAsyncKeyState(VK_TAB)&0x8000);if(down&&!held&&GetForegroundWindow()==target)PostMessageW(target,WM_MLAUNCHER_TOGGLE,0,0);held=down;}}).detach();
            std::thread([value=std::string(version)]{nativeStatus("OpenGL оверлей готов · Shift+Tab · "+value);}).detach();
            wchar_t test[8]{};if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_TEST",test,8)&&test[0]==L'1'){
                setVisible(true);
                wchar_t query[257]{};
                const DWORD n=GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_TEST_QUERY",query,257);
                if(n&&n<257){
                    const int length=WideCharToMultiByte(CP_UTF8,0,query,-1,nullptr,0,nullptr,nullptr);
                    std::string utf8(length,'\0');WideCharToMultiByte(CP_UTF8,0,query,-1,utf8.data(),length,nullptr,nullptr);utf8.pop_back();
                    runJob(L"/resources",{{"query",utf8},{"provider","modrinth"},{"sort","downloads"}});
                }
            }
        }
        if(visible.load()&&current==targetContext&&window==target){
            ImGui::SetCurrentContext(context);
            ImGui_ImplOpenGL3_NewFrame();ImGui_ImplWin32_NewFrame();ImGui::NewFrame();
            drawOverlay();ImGui::Render();
            GLint previousFramebuffer=0;
            if(bindFramebuffer){glGetIntegerv(0x8CA6,&previousFramebuffer);bindFramebuffer(0x8CA9,0);}
            ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
            if(bindFramebuffer)bindFramebuffer(0x8CA9,static_cast<GLuint>(previousFramebuffer));
        }
    }catch(...){setVisible(false);}
    return originalSwap(dc);
}
static DWORD WINAPI initialize(void*) {
    wchar_t portText[16]{},secret[128]{};
    if(!GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_PORT",portText,16)||GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_TOKEN",secret,128)!=32)return 0;
    const unsigned long value=wcstoul(portText,nullptr,10);if(value==0||value>65535)return 0;
    port=static_cast<INTERNET_PORT>(value);token=secret;
    wchar_t style[64]{};
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_ACCENT",style,64)){int r=0,g=0,b=0;if(swscanf_s(style,L"%d,%d,%d",&r,&g,&b)==3)accentColor=ImVec4(r/255.f,g/255.f,b/255.f,1.f);}
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_STYLE",style,64))lightStyle=wcscmp(style,L"light")==0;
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_EFFECT",style,64)){
        if(wcscmp(style,L"Waves")==0)effectStyle=1;else if(wcscmp(style,L"Bubbles")==0)effectStyle=2;
        else if(wcscmp(style,L"Aurora")==0)effectStyle=3;else if(wcscmp(style,L"None")==0)effectStyle=0;
    }
    wchar_t test[8]{};testMode=GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_TEST",test,8)&&test[0]==L'1';
    if(MH_Initialize()!=MH_OK)return 0;
    if(MH_CreateHookApi(L"gdi32.dll","SwapBuffers",reinterpret_cast<void*>(swapHook),reinterpret_cast<void**>(&originalSwap))!=MH_OK||
       MH_CreateHookApi(L"user32.dll","ClipCursor",reinterpret_cast<void*>(clipHook),reinterpret_cast<void**>(&originalClip))!=MH_OK||
       MH_CreateHookApi(L"user32.dll","SetCursorPos",reinterpret_cast<void*>(positionHook),reinterpret_cast<void**>(&originalPosition))!=MH_OK||
       MH_EnableHook(MH_ALL_HOOKS)!=MH_OK){
        MH_DisableHook(MH_ALL_HOOKS);nativeStatus("Не удалось включить перехват OpenGL/ввода; отключите конфликтующие оверлеи.");return 0;
    }
    std::thread([]{Sleep(5000);if(!rendererReady.load())nativeStatus("DLL загружена, но OpenGL-кадр не получен. Vulkan/другой контекст или защита процесса не поддерживаются.");}).detach();
    return 0;
}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayVersion(){return 1;}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayStatus(){return (context?1u:0u)|(visible.load()?2u:0u);}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayResourceCount(){std::lock_guard<std::mutex> lock(dataMutex);return static_cast<unsigned>(data["entries"].size());}
BOOL WINAPI DllMain(HINSTANCE instance,DWORD reason,LPVOID) {
    if(reason==DLL_PROCESS_ATTACH){DisableThreadLibraryCalls(instance);HANDLE thread=CreateThread(nullptr,0,initialize,nullptr,0,nullptr);if(thread)CloseHandle(thread);}
    return TRUE;
}
