Import-Module ./Util.psm1
. "$PSScriptRoot/helpers.ps1"
class Repo : Base { [int] Count() { return 0 } }
function Open-Store {
    param([string]$Path)
    $url = $env:STORE_URL
    Invoke-Helper -Path $Path
    $r = [Repo]::new()
    $r.Count()
}
Describe "Store" { It "opens" { Open-Store -Path "x" | Should -Be $null } }
