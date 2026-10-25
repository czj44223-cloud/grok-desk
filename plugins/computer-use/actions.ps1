$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class DeskInput {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [StructLayout(LayoutKind.Sequential)]
  public struct KEYBDINPUT {
    public ushort wVk;
    public ushort wScan;
    public uint dwFlags;
    public uint time;
    public IntPtr dwExtraInfo;
  }
  [StructLayout(LayoutKind.Explicit)]
  public struct INPUT {
    [FieldOffset(0)] public uint type;
    [FieldOffset(8)] public KEYBDINPUT ki;
  }
  public static void KeyVk(ushort vk, bool up) {
    INPUT input = new INPUT();
    input.type = 1;
    input.ki.wVk = vk;
    input.ki.dwFlags = up ? 2u : 0u;
    INPUT[] list = new INPUT[] { input };
    SendInput(1, list, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void KeyChar(char ch) {
    INPUT down = new INPUT();
    down.type = 1;
    down.ki.wScan = (ushort)ch;
    down.ki.dwFlags = 4;
    INPUT up = down;
    up.ki.dwFlags = 6;
    INPUT[] list = new INPUT[] { down, up };
    SendInput(2, list, Marshal.SizeOf(typeof(INPUT)));
  }
}
"@
[DeskInput]::SetProcessDPIAware() | Out-Null

function Fail([string]$code) {
  [Console]::Error.WriteLine($code)
  exit 1
}

function Screen() {
  $x = [DeskInput]::GetSystemMetrics(76)
  $y = [DeskInput]::GetSystemMetrics(77)
  $w = [DeskInput]::GetSystemMetrics(78)
  $h = [DeskInput]::GetSystemMetrics(79)
  if ($w -le 0 -or $h -le 0) { Fail "NO_SCREEN" }
  return @{ x = $x; y = $y; w = $w; h = $h }
}

function Inside($x, $y, $screen) {
  return ($x -ge $screen.x -and $y -ge $screen.y -and $x -lt ($screen.x + $screen.w) -and $y -lt ($screen.y + $screen.h))
}

$raw = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($raw)) { Fail "BAD_INPUT" }
try { $req = $raw | ConvertFrom-Json } catch { Fail "BAD_INPUT" }
$action = [string]$req.action
$screen = Screen

if ($action -eq "screenshot") {
  $dir = Join-Path $env:TEMP "grok-desk-screen"
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Get-ChildItem $dir -Filter "shot-*.png" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -Skip 6 | Remove-Item -Force -ErrorAction SilentlyContinue
  $bmp = New-Object System.Drawing.Bitmap $screen.w, $screen.h
  $graphics = [System.Drawing.Graphics]::FromImage($bmp)
  $graphics.CopyFromScreen($screen.x, $screen.y, 0, 0, $bmp.Size)
  $graphics.Dispose()
  $limit = 1600
  $scaleX = 1.0
  $scaleY = 1.0
  $image = $bmp
  if ($bmp.Width -gt $limit -or $bmp.Height -gt $limit) {
    $ratio = [Math]::Min($limit / $bmp.Width, $limit / $bmp.Height)
    $nw = [Math]::Max(1, [int]($bmp.Width * $ratio))
    $nh = [Math]::Max(1, [int]($bmp.Height * $ratio))
    $image = New-Object System.Drawing.Bitmap $nw, $nh
    $draw = [System.Drawing.Graphics]::FromImage($image)
    $draw.DrawImage($bmp, 0, 0, $nw, $nh)
    $draw.Dispose()
    $scaleX = [Math]::Round($bmp.Width / $nw, 4)
    $scaleY = [Math]::Round($bmp.Height / $nh, 4)
  }
  $path = Join-Path $dir ("shot-" + [DateTimeOffset]::Now.ToUnixTimeMilliseconds() + ".png")
  $image.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  if (-not [object]::ReferenceEquals($image, $bmp)) { $image.Dispose() }
  $bmp.Dispose()
  $out = @{
    ok = $true
    action = "screenshot"
    path = $path
    originX = $screen.x
    originY = $screen.y
    imageWidth = $image.Width
    imageHeight = $image.Height
    screenWidth = $screen.w
    screenHeight = $screen.h
    scaleX = $scaleX
    scaleY = $scaleY
  }
  [Console]::Out.WriteLine(($out | ConvertTo-Json -Compress))
  exit 0
}

if ($action -eq "windows") {
  $rows = @()
  Get-Process | Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle } | Select-Object -First 30 | ForEach-Object {
    $rows += @{ title = [string]$_.MainWindowTitle; process = [string]$_.ProcessName }
  }
  $out = @{ ok = $true; action = "windows"; windows = $rows }
  [Console]::Out.WriteLine(($out | ConvertTo-Json -Compress -Depth 4))
  exit 0
}

if ($action -eq "click" -or $action -eq "scroll") {
  $x = 0
  $y = 0
  if (-not [double]::TryParse([string]$req.x, [ref]$x) -or -not [double]::TryParse([string]$req.y, [ref]$y)) { Fail "BAD_INPUT" }
  $x = [int]$x
  $y = [int]$y
  if (-not (Inside $x $y $screen)) { Fail "OUT_OF_SCREEN" }
  [DeskInput]::SetCursorPos($x, $y) | Out-Null
  if ($action -eq "scroll") {
    $dy = 0
    if (-not [int]::TryParse([string]$req.dy, [ref]$dy)) { Fail "BAD_INPUT" }
    if ($dy -lt -10 -or $dy -gt 10 -or $dy -eq 0) { Fail "BAD_INPUT" }
    $data = [uint32]([int]($dy * 120) -band 0xffffffff)
    [DeskInput]::mouse_event(0x0800, 0, 0, $data, [UIntPtr]::Zero)
    [Console]::Out.WriteLine('{"ok":true,"action":"scroll"}')
    exit 0
  }
  $button = if ($req.button) { [string]$req.button } else { "left" }
  $clicks = 1
  if ($req.clicks) {
    if (-not [int]::TryParse([string]$req.clicks, [ref]$clicks)) { Fail "BAD_INPUT" }
  }
  if ($clicks -ne 1 -and $clicks -ne 2) { Fail "BAD_INPUT" }
  $down = 0x0002
  $up = 0x0004
  if ($button -eq "right") { $down = 0x0008; $up = 0x0010 }
  elseif ($button -eq "middle") { $down = 0x0020; $up = 0x0040 }
  elseif ($button -ne "left") { Fail "BAD_INPUT" }
  for ($i = 0; $i -lt $clicks; $i++) {
    [DeskInput]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero)
    [DeskInput]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero)
    if ($i -eq 0 -and $clicks -eq 2) { Start-Sleep -Milliseconds 60 }
  }
  [Console]::Out.WriteLine('{"ok":true,"action":"click"}')
  exit 0
}

if ($action -eq "type") {
  $text = [string]$req.text
  if ([string]::IsNullOrEmpty($text)) { Fail "TYPE_EMPTY" }
  if ($text.Length -gt 2000) { Fail "TYPE_TOO_LONG" }
  foreach ($ch in $text.ToCharArray()) {
    if ([int]$ch -lt 32 -or [int]$ch -eq 127) { Fail "BAD_INPUT" }
    [DeskInput]::KeyChar($ch)
  }
  $out = @{ ok = $true; action = "type"; count = $text.Length }
  [Console]::Out.WriteLine(($out | ConvertTo-Json -Compress))
  exit 0
}

if ($action -eq "key") {
  $names = @{
    enter = 0x0D; tab = 0x09; escape = 0x1B; backspace = 0x08; delete = 0x2E
    left = 0x25; up = 0x26; right = 0x27; down = 0x28
    home = 0x24; end = 0x23; pageup = 0x21; pagedown = 0x22; space = 0x20
    f1 = 0x70; f2 = 0x71; f3 = 0x72; f4 = 0x73; f5 = 0x74; f6 = 0x75
    f7 = 0x76; f8 = 0x77; f9 = 0x78; f10 = 0x79; f11 = 0x7A; f12 = 0x7B
  }
  $mods = @{ ctrl = 0x11; alt = 0x12; shift = 0x10 }
  $key = ([string]$req.key).ToLowerInvariant()
  if (-not $names.ContainsKey($key)) { Fail "UNKNOWN_KEY" }
  $held = @()
  if ($req.modifiers) {
    foreach ($item in @($req.modifiers)) {
      $name = ([string]$item).ToLowerInvariant()
      if (-not $mods.ContainsKey($name)) { Fail "UNKNOWN_KEY" }
      $held += $mods[$name]
    }
  }
  foreach ($vk in $held) { [DeskInput]::KeyVk([uint16]$vk, $false) }
  [DeskInput]::KeyVk([uint16]$names[$key], $false)
  [DeskInput]::KeyVk([uint16]$names[$key], $true)
  foreach ($vk in $held) { [DeskInput]::KeyVk([uint16]$vk, $true) }
  [Console]::Out.WriteLine('{"ok":true,"action":"key"}')
  exit 0
}

Fail "UNKNOWN_ACTION"
