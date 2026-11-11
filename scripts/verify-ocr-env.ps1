#Requires -Version 5.1
<#
.SYNOPSIS
    Verifies OCR environment readiness for document-searcher.

.DESCRIPTION
    Checks for:
    - Tesseract OCR (tesseract.exe)
    - Poppler pdftoppm (pdftoppm.exe)
    - DOCUMENT_SEARCHER_OCR_LANG environment variable
    - WinRT OCR availability (Windows 10+ only)
    - GTK4/libadwaita for desktop feature (optional)

.EXAMPLE
    .\verify-ocr-env.ps1

    Runs all checks and prints a summary report.
#>

param()

# Colors for output
function Write-Ok { param($msg) Write-Host "[OK] " -ForegroundColor Green -NoNewline; Write-Host $msg }
function Write-Warn { param($msg) Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline; Write-Host $msg }
function Write-Err { param($msg) Write-Host "[ERR] " -ForegroundColor Red -NoNewline; Write-Host $msg }
function Write-Info { param($msg) Write-Host "[INFO] " -ForegroundColor Cyan -NoNewline; Write-Host $msg }

$script:results = @{
    Tesseract = $false
    TesseractPath = $null
    TesseractVersion = $null
    TesseractLangs = @()
    Pdftoppm = $false
    PdftoppmPath = $null
    PdftoppmVersion = $null
    OcrLangEnv = $null
    WinrtOcr = $false
    WinrtOcrLangs = @()
    Gtk4 = $false
    Libadwaita = $false
    PkgConfig = $false
}

# ============================================================
# Tesseract OCR
# ============================================================
Write-Host "`n=== Tesseract OCR ===" -ForegroundColor White

$tesseract = Get-Command tesseract -ErrorAction SilentlyContinue
if ($tesseract) {
    $script:results.Tesseract = $true
    $script:results.TesseractPath = $tesseract.Source

    # Get version
    $tessVersion = & tesseract --version 2>&1 | Select-Object -First 1
    if ($tessVersion -match "tesseract\s+v?([\d.]+)") {
        $script:results.TesseractVersion = $Matches[1]
        Write-Ok "Tesseract v$($Matches[1]) found at $($tesseract.Source)"
    } else {
        Write-Ok "Tesseract found at $($tesseract.Source)"
    }

    # Get available languages
    $tessLangs = & tesseract --list-langs 2>&1
    if ($LASTEXITCODE -eq 0) {
        $langs = $tessLangs | Where-Object { $_ -match "^[a-z]{3}(_\w+)?$" } | ForEach-Object { $_.Trim() }
        $script:results.TesseractLangs = $langs
        Write-Info "Available languages: $($langs -join ', ')"
    }
} else {
    Write-Err "Tesseract not found in PATH"
    Write-Info "Install via: choco install tesseract -y"
    Write-Info "Or download from: https://github.com/UB-Mannheim/tesseract/wiki"
}

# ============================================================
# Poppler pdftoppm
# ============================================================
Write-Host "`n=== Poppler pdftoppm ===" -ForegroundColor White

$pdftoppm = Get-Command pdftoppm -ErrorAction SilentlyContinue
if ($pdftoppm) {
    $script:results.Pdftoppm = $true
    $script:results.PdftoppmPath = $pdftoppm.Source

    # Get version
    $popplerVersion = & pdftoppm -v 2>&1
    if ($popplerVersion -match "pdftoppm version ([\d.]+)") {
        $script:results.PdftoppmVersion = $Matches[1]
        Write-Ok "pdftoppm v$($Matches[1]) found at $($pdftoppm.Source)"
    } else {
        Write-Ok "pdftoppm found at $($pdftoppm.Source)"
    }
} else {
    Write-Err "pdftoppm not found in PATH"
    Write-Info "Install via: choco install poppler -y"
    Write-Info "Or download from: https://blog.alivate.com.au/poppler-windows/"
}

# ============================================================
# DOCUMENT_SEARCHER_OCR_LANG
# ============================================================
Write-Host "`n=== OCR Language Config ===" -ForegroundColor White

$ocrLang = $env:DOCUMENT_SEARCHER_OCR_LANG
if ($ocrLang) {
    $script:results.OcrLangEnv = $ocrLang
    Write-Ok "DOCUMENT_SEARCHER_OCR_LANG = '$ocrLang'"

    # Verify languages are installed
    if ($script:results.Tesseract) {
        $requestedLangs = $ocrLang -split '\+' | ForEach-Object { $_.Trim() }
        $missingLangs = $requestedLangs | Where-Object { $_ -notin $script:results.TesseractLangs }
        if ($missingLangs) {
            Write-Warn "Missing Tesseract languages: $($missingLangs -join ', ')"
            Write-Info "Download from: https://github.com/tesseract-ocr/tessdata"
        }
    }
} else {
    Write-Warn "DOCUMENT_SEARCHER_OCR_LANG not set"
    Write-Info "Set via: `$env:DOCUMENT_SEARCHER_OCR_LANG = 'rus+eng'"
    Write-Info "Or add to system environment variables"
}

# ============================================================
# WinRT OCR (Windows 10+)
# ============================================================
Write-Host "`n=== WinRT OCR (Windows 10+ fallback) ===" -ForegroundColor White

if ($PSVersionTable.Platform -eq "Win32NT" -or $null -eq $PSVersionTable.Platform) {
    try {
        Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop

        # Check if Windows.Globalization.Language is available
        $langType = [Windows.Globalization.Language]

        # Try to access OCR engine
        $ocrEngineType = [Windows.Media.Ocr.OcrEngine]
        $availableLangs = $ocrEngineType::AvailableRecognizerLanguages

        if ($availableLangs.Count -gt 0) {
            $script:results.WinrtOcr = $true
            $script:results.WinrtOcrLangs = $availableLangs | ForEach-Object { $_.LanguageTag }
            Write-Ok "WinRT OCR available"
            Write-Info "Recognizer languages: $($script:results.WinrtOcrLangs -join ', ')"
        } else {
            Write-Warn "WinRT OCR present but no recognizer languages installed"
            Write-Info "Add language packs via Settings > Time & Language > Language"
        }
    } catch {
        Write-Warn "WinRT OCR not available (requires Windows 10+)"
    }
} else {
    Write-Info "Skipped (not Windows)"
}

# ============================================================
# GTK4/libadwaita (optional, for desktop feature)
# ============================================================
Write-Host "`n=== GTK4/libadwaita (optional, for --features desktop) ===" -ForegroundColor White

$gtk4 = Get-Command gtk4-demo -ErrorAction SilentlyContinue
$libadwaita = Get-Command adwaita-1-demo -ErrorAction SilentlyContinue
$pkgconfig = Get-Command pkg-config -ErrorAction SilentlyContinue

if ($gtk4) {
    $script:results.Gtk4 = $true
    Write-Ok "GTK4 found at $($gtk4.Source)"
} else {
    Write-Warn "GTK4 not found"
}

if ($libadwaita) {
    $script:results.Libadwaita = $true
    Write-Ok "libadwaita found at $($libadwaita.Source)"
} else {
    Write-Warn "libadwaita not found"
}

if ($pkgconfig) {
    $script:results.PkgConfig = $true
    Write-Ok "pkg-config found at $($pkgconfig.Source)"
} else {
    Write-Warn "pkg-config not found"
}

if (-not $script:results.Gtk4 -or -not $script:results.Libadwaita -or -not $script:results.PkgConfig) {
    Write-Info "Install via MSYS2:"
    Write-Info "  pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita mingw-w64-x86_64-pkgconf"
    Write-Info "Then add C:\msys64\mingw64\bin to PATH"
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n=== Summary ===" -ForegroundColor White

$ocrReady = $script:results.Tesseract -and $script:results.Pdftoppm -and $script:results.OcrLangEnv
$winrtReady = $script:results.WinrtOcr
$anyOcrReady = $ocrReady -or $winrtReady
$gtkReady = $script:results.Gtk4 -and $script:results.Libadwaita -and $script:results.PkgConfig

Write-Host ""
Write-Host "OCR Readiness:" -ForegroundColor White
if ($ocrReady) {
    Write-Host "  [FULL] " -ForegroundColor Green -NoNewline
    Write-Host "Tesseract + pdftoppm configured"
} elseif ($winrtReady) {
    Write-Host "  [PARTIAL] " -ForegroundColor Yellow -NoNewline
    Write-Host "WinRT OCR available (Tesseract/pdftoppm not configured)"
} else {
    Write-Host "  [NONE] " -ForegroundColor Red -NoNewline
    Write-Host "No OCR backend available"
}

Write-Host ""
Write-Host "Desktop Feature (GTK):" -ForegroundColor White
if ($gtkReady) {
    Write-Host "  [READY] " -ForegroundColor Green -NoNewline
    Write-Host "GTK4 + libadwaita + pkg-config available"
} else {
    Write-Host "  [NOT READY] " -ForegroundColor Yellow -NoNewline
    Write-Host "Missing GTK4/libadwaita/pkg-config (optional, Linux GUI only)"
}

Write-Host ""
Write-Host "Next Steps:" -ForegroundColor White

if (-not $script:results.Tesseract) {
    Write-Host "  1. Install Tesseract: choco install tesseract -y"
}
if (-not $script:results.Pdftoppm) {
    Write-Host "  2. Install Poppler: choco install poppler -y"
}
if (-not $script:results.OcrLangEnv) {
    Write-Host "  3. Set OCR language: `$env:DOCUMENT_SEARCHER_OCR_LANG = 'rus+eng'"
}
if ($script:results.OcrLangEnv -and $script:results.Tesseract) {
    $requestedLangs = $script:results.OcrLangEnv -split '\+' | ForEach-Object { $_.Trim() }
    $missingLangs = $requestedLangs | Where-Object { $_ -notin $script:results.TesseractLangs }
    if ($missingLangs) {
        Write-Host "  4. Download missing Tesseract languages: $($missingLangs -join ', ')"
    }
}
if (-not $anyOcrReady) {
    Write-Host "  5. Alternatively, ensure Windows language packs are installed for WinRT OCR"
}

Write-Host ""

# Return object for programmatic use
return [PSCustomObject]$script:results
