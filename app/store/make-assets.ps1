# Draws the Store package's images from the app icon, into Assets/. Run it
# again whenever the icon changes, and commit what it writes.
#
# Where icon.ico has the size wanted, its picture is used as it is, because
# its small sizes were drawn by hand. Other sizes are scaled down from the
# large picture.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$icons = Join-Path $PSScriptRoot "..\..\assets\icon"
$out = Join-Path $PSScriptRoot "Assets"
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter *.png | Remove-Item

# every picture inside the .ico, by its width
$ico = [IO.File]::ReadAllBytes((Join-Path $icons "icon.ico"))
$frames = @{}
for ($i = 0; $i -lt [BitConverter]::ToUInt16($ico, 4); $i++) {
    $entry = 6 + 16 * $i
    $width = if ($ico[$entry] -eq 0) { 256 } else { [int]$ico[$entry] }
    $length = [BitConverter]::ToUInt32($ico, $entry + 8)
    $start = [BitConverter]::ToUInt32($ico, $entry + 12)
    $bytes = New-Object byte[] $length
    [Array]::Copy($ico, $start, $bytes, 0, $length)
    $frames[$width] = $bytes
}

function Picture([int]$size) {
    if ($frames.ContainsKey($size)) {
        return [System.Drawing.Image]::FromStream((New-Object IO.MemoryStream (, $frames[$size])))
    }
    $large = [System.Drawing.Image]::FromFile((Join-Path $icons "icon-1024.png"))
    $small = New-Object System.Drawing.Bitmap $size, $size
    $drawing = [System.Drawing.Graphics]::FromImage($small)
    $drawing.InterpolationMode = "HighQualityBicubic"
    $drawing.PixelOffsetMode = "HighQuality"
    $drawing.DrawImage($large, 0, 0, $size, $size)
    $drawing.Dispose()
    $large.Dispose()
    $small
}

# the icon at `icon` pixels, in the middle of a clear square of `canvas` pixels
function Save([string]$name, [int]$canvas, [int]$icon) {
    $square = New-Object System.Drawing.Bitmap $canvas, $canvas
    $drawing = [System.Drawing.Graphics]::FromImage($square)
    $picture = Picture $icon
    $edge = [int](($canvas - $icon) / 2)
    $drawing.DrawImage($picture, $edge, $edge, $icon, $icon)
    $drawing.Dispose()
    $picture.Dispose()
    $square.Save((Join-Path $out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $square.Dispose()
}

# taskbar, Start and Settings, with and without a plate behind the icon
foreach ($size in 16, 24, 32, 48, 256) {
    Save "Square44x44Logo.targetsize-$size" $size $size
    Save "Square44x44Logo.targetsize-${size}_altform-unplated" $size $size
}
Save "Square44x44Logo.scale-100" 44 40
Save "Square44x44Logo.scale-200" 88 80
# the Start tile: the icon with room around it
Save "Square150x150Logo.scale-100" 150 96
Save "Square150x150Logo.scale-200" 300 192
# the Store's own listing
Save "StoreLogo.scale-100" 50 48
Save "StoreLogo.scale-200" 100 96

Get-ChildItem $out -Filter *.png | ForEach-Object { "{0,-60} {1,6} bytes" -f $_.Name, $_.Length }
