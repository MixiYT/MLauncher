// Isolated OpenGL smoke host: never attaches to an existing Minecraft/user process.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shellapi.h>
#include <GL/gl.h>
#include <vector>
#include <fstream>
#include <cstdio>
using Status=unsigned(*)();
static unsigned gameKeys=0,gameMouse=0;
static LRESULT CALLBACK windowProc(HWND window,UINT msg,WPARAM w,LPARAM l){
    if(msg==WM_KEYDOWN&&w==VK_F12)++gameKeys;
    if(msg==WM_MOUSEMOVE)++gameMouse;
    if(msg==WM_CLOSE){DestroyWindow(window);return 0;}
    if(msg==WM_DESTROY){PostQuitMessage(0);return 0;}
    return DefWindowProcW(window,msg,w,l);
}
static void toggle(HWND window,bool repeat=false){
    BYTE saved[256],keys[256];GetKeyboardState(saved);memcpy(keys,saved,256);keys[VK_SHIFT]=0x80;SetKeyboardState(keys);
    SendMessageW(window,WM_KEYDOWN,VK_TAB,repeat?(1u<<30):0);SetKeyboardState(saved);
}
static void screenshot(const wchar_t* path,int w,int h){
    const int stride=(w*3+3)&~3;std::vector<unsigned char> rgb(w*h*3),bgr(stride*h);
    glReadBuffer(GL_FRONT);glPixelStorei(GL_PACK_ALIGNMENT,1);glReadPixels(0,0,w,h,GL_RGB,GL_UNSIGNED_BYTE,rgb.data());
    for(int y=0;y<h;++y)for(int x=0;x<w;++x)for(int c=0;c<3;++c)bgr[y*stride+x*3+c]=rgb[(y*w+x)*3+2-c];
    BITMAPFILEHEADER file{};file.bfType=0x4d42;file.bfOffBits=sizeof(file)+sizeof(BITMAPINFOHEADER);file.bfSize=file.bfOffBits+static_cast<DWORD>(bgr.size());
    BITMAPINFOHEADER info{};info.biSize=sizeof(info);info.biWidth=w;info.biHeight=h;info.biPlanes=1;info.biBitCount=24;
    HANDLE out=CreateFileW(path,GENERIC_WRITE,0,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);DWORD n;
    if(out!=INVALID_HANDLE_VALUE){WriteFile(out,&file,sizeof(file),&n,nullptr);WriteFile(out,&info,sizeof(info),&n,nullptr);WriteFile(out,bgr.data(),static_cast<DWORD>(bgr.size()),&n,nullptr);CloseHandle(out);}
}
int main(){
    int argc=0;wchar_t** argv=CommandLineToArgvW(GetCommandLineW(),&argc);
    WNDCLASSW klass{};klass.style=CS_OWNDC;klass.lpfnWndProc=windowProc;klass.hInstance=GetModuleHandleW(nullptr);klass.lpszClassName=L"GLFW30";RegisterClassW(&klass);
    HWND window=CreateWindowW(L"GLFW30",L"MLauncher OpenGL overlay smoke host",WS_OVERLAPPEDWINDOW,100,100,1100,760,nullptr,nullptr,klass.hInstance,nullptr);
    HDC dc=GetDC(window);PIXELFORMATDESCRIPTOR format{};format.nSize=sizeof(format);format.nVersion=1;format.dwFlags=PFD_DRAW_TO_WINDOW|PFD_SUPPORT_OPENGL|PFD_DOUBLEBUFFER;format.iPixelType=PFD_TYPE_RGBA;format.cColorBits=32;
    int pixelFormat=ChoosePixelFormat(dc,&format);SetPixelFormat(dc,pixelFormat,&format);HGLRC context=wglCreateContext(dc);wglMakeCurrent(dc,context);
    printf("OpenGL smoke host: %s\n",glGetString(GL_VERSION));fflush(stdout);
    const ULONGLONG start=GetTickCount64();ULONGLONG ready=0;bool closed=false,reopened=false,repeatTest=false,escapeTest=false,captured=false,inputCaptured=false,inputRestored=false;int result=1;
    bool shown=false,recreated=false,recreationOK=false,hiddenIgnored=true;
    bool fullscreen=false,fullscreenClosed=false,fullscreenHotkey=false,windowedAgain=false;
    RECT windowed{};
    MONITORINFO monitor{sizeof(MONITORINFO)};
    const bool needsSearch=GetEnvironmentVariableW(L"MLAUNCHER_OVERLAY_TEST_QUERY",nullptr,0)>0;
    for(;;){
        MSG message;while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)){if(message.message==WM_QUIT)return result;TranslateMessage(&message);DispatchMessageW(&message);}
        RECT r;GetClientRect(window,&r);glViewport(0,0,r.right,r.bottom);glClearColor(16.f/255,24.f/255,32.f/255,1);glClear(GL_COLOR_BUFFER_BIT);SwapBuffers(dc);
        const auto total=GetTickCount64()-start;
        HMODULE overlay=GetModuleHandleW(L"mlauncher-overlay.dll");Status status=overlay?reinterpret_cast<Status>(GetProcAddress(overlay,"MLauncherOverlayStatus")):nullptr;
        Status count=overlay?reinterpret_cast<Status>(GetProcAddress(overlay,"MLauncherOverlayResourceCount")):nullptr;
        if(!shown&&total<700&&status&&status()!=0)hiddenIgnored=false;
        if(!shown&&total>=700){ShowWindow(window,SW_SHOW);SetForegroundWindow(window);shown=true;}
        if(!ready&&status&&status()==3&&(!needsSearch||(count&&count()>0))){
            ready=GetTickCount64();const auto keys=gameKeys,mouse=gameMouse;
            SendMessageW(window,WM_KEYDOWN,VK_F12,0);SendMessageW(window,WM_KEYUP,VK_F12,0);SendMessageW(window,WM_MOUSEMOVE,0,MAKELPARAM(100,100));
            inputCaptured=gameKeys==keys&&gameMouse==mouse;printf("Overlay captures input: %s\n",inputCaptured?"OK":"FAIL");
        }
        const auto elapsed=ready?GetTickCount64()-ready:0;
        if(elapsed>600&&!closed&&status&&status()==3){
            toggle(window);closed=status()==1;printf("Hotkey closes overlay: %s\n",closed?"OK":"FAIL");
            const auto keys=gameKeys,mouse=gameMouse;SendMessageW(window,WM_KEYDOWN,VK_F12,0);SendMessageW(window,WM_KEYUP,VK_F12,0);SendMessageW(window,WM_MOUSEMOVE,0,MAKELPARAM(100,100));
            inputRestored=gameKeys==keys+1&&gameMouse==mouse+1;printf("Closing restores game input: %s\n",inputRestored?"OK":"FAIL");
        }
        if(elapsed>1100&&closed&&!reopened&&status){toggle(window);reopened=status()==3;printf("Hotkey reopens overlay: %s\n",reopened?"OK":"FAIL");}
        if(elapsed>1500&&reopened&&!repeatTest&&status){
            toggle(window,true);repeatTest=status()==3;printf("Key repeat does not toggle: %s\n",repeatTest?"OK":"FAIL");
            SendMessageW(window,WM_KEYDOWN,VK_ESCAPE,0);const bool escaped=status()==1;toggle(window);
            escapeTest=escaped&&status()==3;printf("Escape closes and Shift+Tab reopens: %s\n",escapeTest?"OK":"FAIL");
        }
        if(elapsed>1800&&!recreated){
            wglMakeCurrent(nullptr,nullptr);wglDeleteContext(context);context=wglCreateContext(dc);wglMakeCurrent(dc,context);
            recreated=true;
        }else if(recreated&&status&&status()==3)recreationOK=true;
        if(elapsed>2200&&recreationOK&&!fullscreen){
            if(!GetWindowRect(window,&windowed)||
                !GetMonitorInfoW(MonitorFromWindow(window,MONITOR_DEFAULTTONEAREST),&monitor))break;
            const RECT bounds=monitor.rcMonitor;
            SetWindowLongPtrW(window,GWL_STYLE,WS_POPUP|WS_VISIBLE);
            if(!SetWindowPos(window,nullptr,bounds.left,bounds.top,bounds.right-bounds.left,
                bounds.bottom-bounds.top,SWP_FRAMECHANGED|SWP_NOZORDER))break;
            fullscreen=true;
        }
        if(elapsed>2600&&fullscreen&&!fullscreenClosed&&status&&status()==3&&
            r.right==monitor.rcMonitor.right-monitor.rcMonitor.left&&
            r.bottom==monitor.rcMonitor.bottom-monitor.rcMonitor.top){
            toggle(window);
            fullscreenClosed=status()==1;
            if(!fullscreenClosed)break;
        }
        if(elapsed>2850&&fullscreenClosed&&!fullscreenHotkey&&status&&status()==1){
            toggle(window);
            fullscreenHotkey=status()==3;
            if(!fullscreenHotkey)break;
        }
        if(elapsed>3000&&fullscreenHotkey&&!windowedAgain){
            SetWindowLongPtrW(window,GWL_STYLE,WS_OVERLAPPEDWINDOW|WS_VISIBLE);
            if(!SetWindowPos(window,nullptr,windowed.left,windowed.top,windowed.right-windowed.left,
                windowed.bottom-windowed.top,SWP_FRAMECHANGED|SWP_NOZORDER))break;
            windowedAgain=true;
        }
        if(elapsed>3300&&windowedAgain&&hiddenIgnored&&recreationOK&&!captured&&reopened&&repeatTest&&escapeTest&&inputCaptured&&inputRestored&&status&&status()==3){if(argc>1)screenshot(argv[1],r.right,r.bottom);captured=true;result=0;}
        if(elapsed>4000||total>25000)break;Sleep(16);
    }
    printf("Hidden bootstrap context ignored: %s\n",hiddenIgnored?"OK":"FAIL");
    printf("Context recreation restores rendering: %s\n",recreationOK?"OK":"FAIL");
    printf("Borderless fullscreen hotkey and return to windowed: %s\n",fullscreenHotkey&&windowedAgain?"OK":"FAIL");
    printf("Native injected rendering + hotkeys: %s\n",result==0?"OK":"FAIL");fflush(stdout);
    wglMakeCurrent(nullptr,nullptr);wglDeleteContext(context);
    ReleaseDC(window,dc);DestroyWindow(window);
    // The DLL stays loaded for the child process lifetime.
    return result;
}
