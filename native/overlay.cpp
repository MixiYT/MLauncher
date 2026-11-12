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
#include <unordered_map>
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
using DeleteContextFn=BOOL(WINAPI*)(HGLRC);
using ClipFn=BOOL(WINAPI*)(const RECT*);
using PositionFn=BOOL(WINAPI*)(int,int);
static SwapFn originalSwap=nullptr;
static DeleteContextFn originalDeleteContext=nullptr;
static std::recursive_mutex rendererMutex;
static HDC targetDC=nullptr;
static bool resumeVisible=false;
static ClipFn originalClip=nullptr;
static PositionFn originalPosition=nullptr;
static std::atomic<HWND> target{nullptr};
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
static const UINT WM_MLAUNCHER_TOGGLE=WM_APP+1;
static ULONGLONG lastHotkeyToggle=0;
// Both the window event and polling fallback can report the same press.
// Only the window thread toggles visibility; suppress duplicate reports.
static void hotkeyToggle();
using BindFramebufferFn=void(APIENTRY*)(GLenum,GLuint);
static BindFramebufferFn bindFramebuffer=nullptr;
static ImVec4 accentColor(0.47f,0.73f,0.67f,1.f);
static bool lightStyle=false;
static std::wstring themeName=L"Legacy";
static std::wstring interfaceName=L"Standard";
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
    ImVec4 base=lightStyle?ImVec4(.93f,.94f,.96f,.97f):ImVec4(.18f,.24f,.26f,.97f);
    ImVec4 frame=lightStyle?ImVec4(.85f,.88f,.91f,1):ImVec4(.27f,.31f,.32f,1);
    if(themeName==L"Dark"){base=ImVec4(.11f,.12f,.15f,.97f);frame=ImVec4(.18f,.19f,.23f,1.f);}
    if(themeName==L"Amoled"){base=ImVec4(.016f,.016f,.028f,1.f);frame=ImVec4(.08f,.08f,.10f,1.f);}
    if(interfaceName==L"Material"){
        style.WindowRounding=18.f;style.FrameRounding=12.f;style.TabRounding=10.f;
        style.WindowBorderSize=0.f;
    }else if(interfaceName==L"LiquidGlass"){
        style.WindowRounding=20.f;style.FrameRounding=14.f;style.TabRounding=12.f;
        style.FrameBorderSize=1.f;base.w=.76f;frame.w=.68f;
    }else if(interfaceName==L"Simple"){
        style.WindowRounding=2.f;style.FrameRounding=2.f;style.TabRounding=2.f;
        style.FrameBorderSize=1.f;base.w=1.f;
    }
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
static void hotkeyToggle() {
    const ULONGLONG now=GetTickCount64();
    if(lastHotkeyToggle&&now-lastHotkeyToggle<150)return;
    lastHotkeyToggle=now;setVisible(!visible.load());
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
    std::lock_guard<std::recursive_mutex> guard(rendererMutex);
    if(message==WM_MLAUNCHER_TOGGLE){hotkeyToggle();return 0;}
    // Handle real key events as well as polling, including taps shorter than 40 ms.
    if(message==WM_KEYDOWN&&w==VK_TAB&&(GetKeyState(VK_SHIFT)&0x8000)&&!(l&(1u<<30))){hotkeyToggle();return 0;}
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
            case WM_SETCURSOR:SetCursor(nullptr);return TRUE;
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
            else if(path==L"/resources" || path==L"/maps"){
                data["maps"] = path == L"/maps";
                data["catalog_revision"] = data.value("catalog_revision", 0u) + 1;
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
static std::unordered_map<std::string, GLuint> catalogTextures;
static unsigned textureRevision = 0;

static void clearCatalogTextures() {
    for (const auto& item : catalogTextures) glDeleteTextures(1, &item.second);
    catalogTextures.clear();
}

static GLuint catalogTexture(const json& entry) {
    const auto id = entry.value("id", std::string());
    if (auto it = catalogTextures.find(id); it != catalogTextures.end()) return it->second;
    const auto icon = entry.find("icon");
    if (icon == entry.end() || !icon->is_string()) return 0;
    const auto& hex = icon->get_ref<const std::string&>();
    unsigned char rgba[32 * 32 * 4];
    if (hex.size() != sizeof(rgba) * 2) return 0;
    const auto digit = [](char c) {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return c - 'a' + 10;
        return -1;
    };
    for (size_t i = 0; i < sizeof(rgba); ++i) {
        const int high = digit(hex[i * 2]), low = digit(hex[i * 2 + 1]);
        if (high < 0 || low < 0) return 0;
        rgba[i] = static_cast<unsigned char>(high * 16 + low);
    }
    GLint previous = 0;
    glGetIntegerv(GL_TEXTURE_BINDING_2D, &previous);
    GLuint texture = 0;
    glGenTextures(1, &texture);
    glBindTexture(GL_TEXTURE_2D, texture);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 32, 32, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba);
    glBindTexture(GL_TEXTURE_2D, static_cast<GLuint>(previous));
    catalogTextures.emplace(id, texture);
    return texture;
}

static void drawCatalog(const json& snapshot, bool maps) {
    static char resourceQuery[257]{}, mapQuery[257]{};
    static int provider = 0, sort = 0, mapSort = 0;
    char* query = maps ? mapQuery : resourceQuery;
    int& order = maps ? mapSort : sort;
    if (!maps) {
        ImGui::SetNextItemWidth(140);
        ImGui::Combo("##provider", &provider, "Modrinth\0CurseForge\0");
        ImGui::SameLine();
    }
    ImGui::SetNextItemWidth(190);
    ImGui::Combo("##sort", &order, "По скачиваниям\0Релевантность\0Обновлённые\0Новые\0");
    ImGui::SetNextItemWidth(-85);
    bool search = ImGui::InputTextWithHint("##search", maps ? "Найти карту" : "Найти ресурспак",
        query, 257, ImGuiInputTextFlags_EnterReturnsTrue);
    ImGui::SameLine();
    ImGui::BeginDisabled(busy.load());
    search = ImGui::Button("Найти") || search;
    ImGui::EndDisabled();
    if (search && !busy.load()) {
        static const char* sorts[] = {"downloads", "relevance", "updated", "newest"};
        runJob(maps ? L"/maps" : L"/resources", {
            {"query", query}, {"provider", maps || provider == 1 ? "curseforge" : "modrinth"},
            {"sort", sorts[order]}
        });
    }
    const auto status = snapshot.value("status", std::string());
    if (!status.empty()) ImGui::TextWrapped("%s", status.c_str());
    if (snapshot.value("maps", false) != maps) return;
    const unsigned revision = snapshot.value("catalog_revision", 0u);
    if (textureRevision != revision) {
        clearCatalogTextures();
        textureRevision = revision;
    }
    ImGui::BeginChild("cards", ImVec2(0, 0), true);
    for (const auto& entry : snapshot.at("entries")) {
        const auto id = entry.value("id", std::string());
        ImGui::PushID(id.c_str());
        const GLuint texture = catalogTexture(entry);
        if (texture) ImGui::Image(static_cast<ImTextureID>(texture), ImVec2(48, 48));
        else ImGui::Dummy(ImVec2(48, 48));
        ImGui::SameLine();
        ImGui::BeginGroup();
        ImGui::TextWrapped("%s", entry.value("name", std::string()).c_str());
        ImGui::TextDisabled("%s", entry.value("author", std::string()).c_str());
        ImGui::TextWrapped("%s", entry.value("description", std::string()).c_str());
        const bool installed = entry.value("installed", false);
        ImGui::BeginDisabled(busy.load() || installed);
        if (ImGui::Button(installed ? "Установлено" : "Скачать")) {
            runJob(maps ? L"/download-map" : L"/download", {
                {"id", id}, {"provider", snapshot.value("provider", std::string("modrinth"))}
            });
        }
        ImGui::EndDisabled();
        ImGui::EndGroup();
        ImGui::Separator();
        ImGui::PopID();
    }
    ImGui::EndChild();
}

static void drawOverlay() {
    static json snapshot;static unsigned lastRevision=0;
    if(lastRevision!=revision.load()){
        std::lock_guard<std::mutex> lock(dataMutex);snapshot=data;lastRevision=revision.load();
    }
    const ImVec2 screen=ImGui::GetIO().DisplaySize;
    static ImVec2 previousScreen(0.f, 0.f);
    const bool resized=screen.x!=previousScreen.x||screen.y!=previousScreen.y;
    previousScreen=screen;
    const ImGuiCond placement=resized?ImGuiCond_Always:ImGuiCond_FirstUseEver;
    drawEffectBackground(screen);
    ImGui::GetBackgroundDrawList()->AddRectFilled(ImVec2(0,0),screen,lightStyle?IM_COL32(220,226,232,70):IM_COL32(0,0,0,140));
    ImGui::SetNextWindowSize(ImVec2(std::min(920.f,screen.x*.90f),std::min(680.f,screen.y*.9f)),placement);
    ImGui::SetNextWindowPos(ImVec2(screen.x*.5f,screen.y*.5f),placement,ImVec2(.5f,.5f));
    bool open=true;
    if(ImGui::Begin("MLauncher · Shift+Tab / Esc",&open,ImGuiWindowFlags_NoCollapse)){
        ImGui::TextUnformatted(snapshot.value("name",std::string("Minecraft")).c_str());
        if(ImGui::BeginTabBar("overlay-tabs")){
            if (ImGui::BeginTabItem("Ресурспаки")) {
                drawCatalog(snapshot, false);
                ImGui::EndTabItem();
            }
            if (ImGui::BeginTabItem("Карты")) {
                drawCatalog(snapshot, true);
                ImGui::EndTabItem();
            }
            if(ImGui::BeginTabItem("Журнал")){
                static bool follow=true;ImGui::Checkbox("Прокручивать вниз",&follow);
                ImGui::BeginChild("logs",ImVec2(0,0),true,ImGuiWindowFlags_HorizontalScrollbar);
                const bool atBottom=ImGui::GetScrollY()>=ImGui::GetScrollMaxY()-2.f;
                if(const auto logs=snapshot.find("logs");logs!=snapshot.end()&&logs->is_array()){
                    ImGuiListClipper clipper;
                    clipper.Begin(static_cast<int>(logs->size()),ImGui::GetTextLineHeightWithSpacing());
                    while(clipper.Step())for(int row=clipper.DisplayStart;row<clipper.DisplayEnd;++row){
                        const auto& line=(*logs)[row];
                        ImGui::TextUnformatted(line.is_string()?line.get_ref<const std::string&>().c_str():"");
                    }
                }
                if(follow&&atBottom)ImGui::SetScrollHereY(1.f);
                ImGui::EndChild();ImGui::EndTabItem();
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
// Release GL objects while their owning context still exists. Sodium and fullscreen
// transitions may replace that context while keeping the same GLFW window.
static bool releaseGL() {
    if(!context||!targetContext)return true;
    const HGLRC saved=wglGetCurrentContext();const HDC savedDC=wglGetCurrentDC();
    if(saved!=targetContext&&!wglMakeCurrent(targetDC,targetContext))return false;
    ImGui::SetCurrentContext(context);
    clearCatalogTextures();
    ImGui_ImplOpenGL3_Shutdown();
    if(saved!=targetContext)wglMakeCurrent(savedDC,saved);
    targetContext=nullptr;rendererReady.store(false);
    return true;
}
static BOOL WINAPI deleteContextHook(HGLRC gl) {
    std::lock_guard<std::recursive_mutex> guard(rendererMutex);
    if(gl==targetContext){
        resumeVisible=visible.load();setVisible(false);
        if(!releaseGL()){
            // WGL refuses to delete a context still current on another thread.
            // Keep its renderer usable if the caller retries deletion later.
            setVisible(resumeVisible);resumeVisible=false;
        }
    }
    return originalDeleteContext(gl);
}
static BOOL WINAPI swapHook(HDC dc) {
    std::lock_guard<std::recursive_mutex> guard(rendererMutex);
    const HWND window=WindowFromDC(dc);const HGLRC current=wglGetCurrentContext();
    // This DLL exists only inside our child Java process. Avoid depending on a window class:
    // launchers/mods may create wrappers, but the game's GL context is still the one to draw.
    RECT client{};
    if(rendererFailed||!window||!current||!IsWindowVisible(window)||!GetClientRect(window,&client)
        ||client.right<64||client.bottom<64)return originalSwap(dc);
    try{
        if(context&&current!=targetContext){
            const bool reopen=resumeVisible||visible.load();
            setVisible(false);
            if(!releaseGL())return originalSwap(dc);
            ImGui::SetCurrentContext(context);
            if(window!=target){
                if(IsWindow(target)&&reinterpret_cast<WNDPROC>(GetWindowLongPtrW(target,GWLP_WNDPROC))==windowProc)
                    SetWindowLongPtrW(target,GWLP_WNDPROC,reinterpret_cast<LONG_PTR>(originalWndProc));
                ImGui_ImplWin32_Shutdown();
                if(!ImGui_ImplWin32_Init(window))return originalSwap(dc);
                target=window;
                originalWndProc=reinterpret_cast<WNDPROC>(SetWindowLongPtrW(window,GWLP_WNDPROC,reinterpret_cast<LONG_PTR>(windowProc)));
            }
            if(!ImGui_ImplOpenGL3_Init("#version 130"))return originalSwap(dc);
            targetContext=current;targetDC=dc;
            bindFramebuffer=reinterpret_cast<BindFramebufferFn>(wglGetProcAddress("glBindFramebuffer"));
            rendererReady.store(true);resumeVisible=false;setVisible(reopen);
            std::thread([]{nativeStatus("OpenGL-контекст оверлея восстановлен · Shift+Tab");}).detach();
        }
        if(!context){
            const char* version=reinterpret_cast<const char*>(glGetString(GL_VERSION));
            if(!version||atoi(version)<3)return originalSwap(dc);
            target=window;targetContext=current;targetDC=dc;
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
            std::thread([]{bool held=false;for(;;){Sleep(40);if(!target||!rendererReady.load())continue;const bool down=(GetAsyncKeyState(VK_SHIFT)&0x8000)&&(GetAsyncKeyState(VK_TAB)&0x8000);if(down&&!held&&GetForegroundWindow()==target)PostMessageW(target,WM_MLAUNCHER_TOGGLE,0,0);held=down;}}).detach();
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
            // Minecraft's texture uploader leaves pixel-store offsets set.
            // ImGui uploads its atlas on the first frame using client memory.
            constexpr GLenum unpackState[] = {
                GL_UNPACK_ALIGNMENT, GL_UNPACK_ROW_LENGTH,
                GL_UNPACK_SKIP_PIXELS, GL_UNPACK_SKIP_ROWS
            };
            constexpr GLint unpackDefaults[] = {4, 0, 0, 0};
            GLint previousUnpack[4]{};
            for (int i = 0; i < 4; ++i) {
                glGetIntegerv(unpackState[i], &previousUnpack[i]);
                glPixelStorei(unpackState[i], unpackDefaults[i]);
            }
            using BindBufferFn = void(APIENTRY*)(GLenum, GLuint);
            const auto bindBuffer = reinterpret_cast<BindBufferFn>(wglGetProcAddress("glBindBuffer"));
            constexpr GLenum unpackBuffer = 0x88EC;
            GLint previousUnpackBuffer = 0;
            if (bindBuffer) {
                glGetIntegerv(0x88EF, &previousUnpackBuffer); // GL_PIXEL_UNPACK_BUFFER_BINDING
                bindBuffer(unpackBuffer, 0);
            }
            ImGui_ImplOpenGL3_NewFrame();


            // GLFW can keep the hardware cursor hidden while a world is loaded.
            ImGui::GetIO().MouseDrawCursor = true;
            ImGui_ImplWin32_NewFrame();
            ImGui::NewFrame();
            drawOverlay();
            if (bindBuffer) bindBuffer(unpackBuffer, static_cast<GLuint>(previousUnpackBuffer));
            for (int i = 0; i < 4; ++i) glPixelStorei(unpackState[i], previousUnpack[i]);
            ImGui::Render();
            GLint previousFramebuffer = 0;
            if (bindFramebuffer) {
                glGetIntegerv(0x8CA6, &previousFramebuffer); // GL_DRAW_FRAMEBUFFER_BINDING
                bindFramebuffer(0x8CA9, 0); // GL_DRAW_FRAMEBUFFER
            }
            GLboolean previousColorMask[4]{};
            glGetBooleanv(GL_COLOR_WRITEMASK, previousColorMask);
            glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
            ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
            glColorMask(previousColorMask[0], previousColorMask[1], previousColorMask[2], previousColorMask[3]);
            if (bindFramebuffer) bindFramebuffer(0x8CA9, static_cast<GLuint>(previousFramebuffer));
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
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_THEME",style,64))themeName=style;
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_INTERFACE",style,64))interfaceName=style;
    if(GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_EFFECT",style,64)){
        if(wcscmp(style,L"Waves")==0)effectStyle=1;else if(wcscmp(style,L"Bubbles")==0)effectStyle=2;
        else if(wcscmp(style,L"Aurora")==0)effectStyle=3;else if(wcscmp(style,L"None")==0)effectStyle=0;
    }
    if(MH_Initialize()!=MH_OK)return 0;
    if(MH_CreateHookApi(L"opengl32.dll","wglDeleteContext",reinterpret_cast<void*>(deleteContextHook),reinterpret_cast<void**>(&originalDeleteContext))!=MH_OK||
       MH_CreateHookApi(L"gdi32.dll","SwapBuffers",reinterpret_cast<void*>(swapHook),reinterpret_cast<void**>(&originalSwap))!=MH_OK||
       MH_CreateHookApi(L"user32.dll","ClipCursor",reinterpret_cast<void*>(clipHook),reinterpret_cast<void**>(&originalClip))!=MH_OK||
       MH_CreateHookApi(L"user32.dll","SetCursorPos",reinterpret_cast<void*>(positionHook),reinterpret_cast<void**>(&originalPosition))!=MH_OK||
       MH_EnableHook(MH_ALL_HOOKS)!=MH_OK){
        MH_DisableHook(MH_ALL_HOOKS);nativeStatus("Не удалось включить перехват OpenGL/ввода; отключите конфликтующие оверлеи.");return 0;
    }
    std::thread([]{Sleep(5000);if(!rendererReady.load())nativeStatus("DLL загружена, но OpenGL-кадр не получен. Vulkan/другой контекст или защита процесса не поддерживаются.");}).detach();
    return 0;
}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayVersion(){return 1;}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayStatus(){return (rendererReady.load()?1u:0u)|(visible.load()?2u:0u);}
extern "C" __declspec(dllexport) unsigned MLauncherOverlayResourceCount(){std::lock_guard<std::mutex> lock(dataMutex);return static_cast<unsigned>(data["entries"].size());}
BOOL WINAPI DllMain(HINSTANCE instance,DWORD reason,LPVOID) {
    if(reason==DLL_PROCESS_ATTACH){DisableThreadLibraryCalls(instance);HANDLE thread=CreateThread(nullptr,0,initialize,nullptr,0,nullptr);if(thread)CloseHandle(thread);}
    else if(reason==DLL_PROCESS_DETACH){
        // OpenGL32 may destroy remaining contexts after this DLL's C++ globals.
        // Restore entry points before that teardown can call a stale hook.
        MH_DisableHook(MH_ALL_HOOKS);
    }
    return TRUE;
}
