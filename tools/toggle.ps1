# Toggle a rule's enable switch (the ToggleSwitch inside a rule card).
#
# ToggleSwitch exposes TogglePattern; we pick the Nth one on the page.

param(
    [string]$ProcessName = "ClashZhengl",
    [int]$Index = 0,
    [int]$WaitMs = 1500
)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Error "not running"; exit 2 }

$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $proc.Id)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) { Write-Error "window not found"; exit 3 }

$toggles = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)))

# Enumerate all elements that support TogglePattern
$all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    [System.Windows.Automation.Condition]::TrueCondition)

$togglable = @()
foreach ($el in $all) {
    try {
        $null = $el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
        $togglable += $el
    } catch { }
}

Write-Host "toggle-capable elements: $($togglable.Count)"
for ($i = 0; $i -lt $togglable.Count; $i++) {
    $state = $togglable[$i].GetCurrentPattern(
        [System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState
    Write-Host ("  [{0}] {1} [{2}] '{3}'" -f $i,
        $togglable[$i].Current.ControlType.ProgrammaticName, $state,
        $togglable[$i].Current.Name)
}

if ($Index -lt $togglable.Count) {
    $pat = $togglable[$Index].GetCurrentPattern(
        [System.Windows.Automation.TogglePattern]::Pattern)
    Write-Host "before: $($pat.Current.ToggleState)"
    $pat.Toggle()
    Start-Sleep -Milliseconds $WaitMs
    Write-Host "after:  $($pat.Current.ToggleState)"
}

Write-Host "done"
