# Drive the app through UI Automation: click a nav item by its visible name.
#
# Usage:
#   powershell -File tools/click.ps1 -Name "Rules"
#   powershell -File tools/click.ps1 -Name "Settings" -Index 0

param(
    [string]$ProcessName = "ClashZhengl",
    [string]$Name = "",
    [int]$Index = 0,
    [int]$WaitMs = 1200
)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Error "process not running"; exit 2 }

$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $proc.Id)

$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) { Write-Error "window not found"; exit 3 }

if (-not $Name) {
    # List every element that exposes a name, to discover the tree.
    $all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($el in $all) {
        $n = $el.Current.Name
        if ($n) {
            Write-Host ("{0,-22} {1}" -f $el.Current.ControlType.ProgrammaticName, $n)
        }
    }
    exit 0
}

$nameCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::NameProperty, $Name)

$matches = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $nameCond)
if ($matches.Count -eq 0) { Write-Error "no element named '$Name'"; exit 4 }

$target = $matches[$Index]

try {
    $pattern = $target.GetCurrentPattern(
        [System.Windows.Automation.SelectionItemPattern]::Pattern)
    $pattern.Select()
    Write-Host "selected '$Name' via SelectionItem"
} catch {
    try {
        $inv = $target.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $inv.Invoke()
        Write-Host "invoked '$Name'"
    } catch {
        Write-Error "element '$Name' is not selectable/invokable"
        exit 5
    }
}

Start-Sleep -Milliseconds $WaitMs
Write-Host "done"
