param([switch]$TestHost)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$native=Join-Path $root 'native'
$downloads=Join-Path $native 'downloads'
$vendor=Join-Path $native 'vendor'
$bin=Join-Path $native 'bin'
$objects=Join-Path $native 'objects'
foreach($dir in @($downloads,$vendor,$bin,$objects)){New-Item -ItemType Directory -Force -Path $dir | Out-Null}
$deps=@(
    @('minhook-v1.3.4.zip','https://codeload.github.com/TsudaKageyu/minhook/zip/refs/tags/v1.3.4','172708123DAA0C98D20D3A980B16A50BE14AF243DC95DEE6F79C24193AD010E4'),
    @('imgui-v1.91.9b.zip','https://codeload.github.com/ocornut/imgui/zip/refs/tags/v1.91.9b','FD37507C8476A6D14CC7C4B352401F31BCBD0F0D995D35390811E968C466F46E'),
    @('json-v3.11.3.hpp','https://raw.githubusercontent.com/nlohmann/json/v3.11.3/single_include/nlohmann/json.hpp','9BEA4C8066EF4A1C206B2BE5A36302F8926F7FDC6087AF5D20B417D0CF103EA6')
)
foreach($dep in $deps){
    $path=Join-Path $downloads $dep[0]
    if(-not(Test-Path $path)){Invoke-WebRequest -UseBasicParsing -Uri $dep[1] -OutFile $path}
    if((Get-FileHash $path -Algorithm SHA256).Hash -ne $dep[2]){throw "Dependency hash mismatch: $path"}
}
Expand-Archive -Path (Join-Path $downloads 'minhook-v1.3.4.zip') -DestinationPath $vendor -Force
Expand-Archive -Path (Join-Path $downloads 'imgui-v1.91.9b.zip') -DestinationPath $vendor -Force
Copy-Item (Join-Path $downloads 'json-v3.11.3.hpp') (Join-Path $vendor 'json.hpp') -Force
$minhook=Join-Path $vendor 'minhook-1.3.4'
$imgui=Join-Path $vendor 'imgui-1.91.9b'
$objectFiles=@()
foreach($file in @('buffer.c','hook.c','trampoline.c','hde\hde64.c')){
    $inputFile=Join-Path $minhook "src\$file"
    $object=Join-Path $objects ((Split-Path $file -Leaf)+'.o')
    & gcc -O2 -DNDEBUG -c $inputFile -o $object
    if($LASTEXITCODE -ne 0){throw "MinHook build failed: $file"}
    $objectFiles+=$object
}
$sources=@('imgui.cpp','imgui_draw.cpp','imgui_widgets.cpp','imgui_tables.cpp','backends\imgui_impl_win32.cpp','backends\imgui_impl_opengl3.cpp') | ForEach-Object {Join-Path $imgui $_}
$dll=Join-Path $bin 'mlauncher-overlay.dll'
& g++ -std=c++17 -O2 -shared -static -static-libgcc -static-libstdc++ -pthread -DNDEBUG -DUNICODE -D_UNICODE -DIMGUI_IMPL_WIN32_DISABLE_GAMEPAD '-Wl,--no-insert-timestamp' -I $imgui -I (Join-Path $minhook 'include') -I $vendor (Join-Path $native 'overlay.cpp') @sources @objectFiles -o $dll -lopengl32 -lgdi32 -luser32 -lwinhttp -ldwmapi -limm32
if($LASTEXITCODE -ne 0){throw 'Overlay DLL build failed'}
& strip --strip-unneeded $dll
if($LASTEXITCODE -ne 0){throw 'Overlay DLL strip failed'}
Copy-Item (Join-Path $minhook 'LICENSE.txt') (Join-Path $bin 'LICENSE-Minhook.txt') -Force
Copy-Item (Join-Path $imgui 'LICENSE.txt') (Join-Path $bin 'LICENSE-ImGui.txt') -Force
Copy-Item (Join-Path $native 'LICENSE-json.txt') $bin -Force
if($TestHost){
    & g++ -std=c++17 -O2 -static -static-libgcc -static-libstdc++ (Join-Path $native 'test-host.cpp') -o (Join-Path $bin 'overlay-test-host.exe') -lopengl32 -lgdi32 -luser32 -lshell32
    if($LASTEXITCODE -ne 0){throw 'Overlay test host build failed'}
}
Write-Host "Overlay DLL: $dll"
Write-Host "SHA256: $((Get-FileHash $dll -Algorithm SHA256).Hash)"
