# Drive the app through UI Automation: select a nav page by its label.
#
# The nav entries expose their label as ControlType.Text; the selectable element
# is the ListViewItem ancestor, so we walk up until we find a SelectionItem.

param(
    [string]$ProcessName = "ClashZhengl",
    [string]$Label = "",
    [int]$WaitMs = 1500
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

if (-not $Label) {
    $items = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::ListItem)))
    Write-Host "list items: $($items.Count)"
    foreach ($i in $items) { Write-Host "  '$($i.Current.Name)'" }
    exit 0
}

$nameCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::NameProperty, $Label)
$found = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $nameCond)
if ($found.Count -eq 0) { Write-Error "label '$Label' not found"; exit 4 }

$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker

$done = $false
foreach ($el in $found) {
    $cur = $el
    for ($depth = 0; $depth -lt 6 -and $cur; $depth++) {
        try {
            $sel = $cur.GetCurrentPattern(
                [System.Windows.Automation.SelectionItemPattern]::Pattern)
            $sel.Select()
            Write-Host "selected nav '$Label' (depth $depth)"
            $done = $true
            break
        } catch { }
        $cur = $walker.GetParent($cur)
    }
    if ($done) { break }
}

if (-not $done) { Write-Error "could not select '$Label'"; exit 5 }

Start-Sleep -Milliseconds $WaitMs
Write-Host "done"
