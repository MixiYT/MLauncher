$ErrorActionPreference = 'Stop'
$backup = @{}
$installed = @()
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $digest) {
        throw 'Update archive checksum mismatch'
    }
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($name in $files) {
            $entry = $zip.GetEntry($name)
            if (!$entry -or $entry.Length -le 0 -or $entry.Length -gt 512MB) {
                throw "Invalid or missing update file: $name"
            }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $stage $name))
        }
    } finally {
        $zip.Dispose()
    }
    $launcher = Get-Process -Id $launcherPid -ErrorAction SilentlyContinue
    if ($launcher -and !$launcher.WaitForExit(30000)) { throw 'Launcher did not exit' }
    foreach ($name in $files) {
        $destination = Join-Path $directory $name
        $previous = Join-Path $stage "$name.previous"
        if (Test-Path -LiteralPath $destination) {
            Move-Item -LiteralPath $destination -Destination $previous
            $backup[$destination] = $previous
        }
        Move-Item -LiteralPath (Join-Path $stage $name) -Destination $destination
        $installed += $destination
    }
} catch {
    $failure = $_
    $installed | ForEach-Object { Remove-Item -LiteralPath $_ -Force -ErrorAction SilentlyContinue }
    $restored = $true
    foreach ($destination in $backup.Keys) {
        try {
            Move-Item -LiteralPath $backup[$destination] -Destination $destination -Force
        } catch {
            $restored = $false
            $_ | Out-String | Add-Content -LiteralPath $log
        }
    }
    $failure | Out-String | Add-Content -LiteralPath $log
    if ($restored -and (Test-Path -LiteralPath $exe)) {
        $launcher = Get-Process -Id $launcherPid -ErrorAction SilentlyContinue
        if (!$launcher -or $launcher.WaitForExit(30000)) {
            Start-Process -FilePath $exe -WorkingDirectory $directory
        }
    }
    exit 1
}
Remove-Item -LiteralPath $log -ErrorAction SilentlyContinue
Start-Process -FilePath $exe -WorkingDirectory $directory
Get-ChildItem -LiteralPath $stage -File | Remove-Item -Force
Remove-Item -LiteralPath $stage
