$ws = New-Object -ComObject WScript.Shell
$desktop = [System.Environment]::GetFolderPath('Desktop')
$sc = $ws.CreateShortcut("$desktop\JupiterOS.lnk")
$sc.TargetPath = "C:\Users\Edoardo\PycharmProjects\Tool-AI\jupiteros\src-tauri\target\debug\jupiteros.exe"
$sc.WorkingDirectory = "C:\Users\Edoardo\PycharmProjects\Tool-AI"
$sc.Save()
Write-Host "Shortcut created on Desktop"
