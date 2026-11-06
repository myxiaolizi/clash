# Invoke a button by its visible label (ControlType.Button, exact name).
#
# Usage:
#   powershell -File tools\press.ps1 -Label "Add"
#   powershell -File tools\press.ps1 -Label "Delete" -Index 1

param(
    [string]$ProcessName = "ClashZhengl",
    [string]$Label = "",
    [int]$Index = 0,
    [int]$WaitMs = 1200
)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Error "process not running"; exit 2 }

$root = [System.Windows.Automation.AutomationElement]::RootElement
$pidCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $proc.Id)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $pidCond)
if (-not $win) { Write-Error "window not found"; exit 3 }

if (-not $Label) {
    $btns = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)))
    Write-Host "buttons: $($btns.Count)"
    foreach ($b in $btns) { Write-Host "  '$($b.Current.Name)'" }
    exit 0
}

$cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)),
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Label)))

$found = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
if ($found.Count -eq 0) { Write-Error "no button '$Label'"; exit 4 }
if ($Index -ge $found.Count) { Write-Error "index $Index out of range ($($found.Count) matches)"; exit 5 }

$inv = $found[$Index].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
$inv.Invoke()
Write-Host "invoked '$Label' [$Index]"
Start-Sleep -Milliseconds $WaitMs
Write-Host "done"
