function Get-Bumped {
    param([int]$n)
    return $n + 1
}

function Add-Total {
    param([int]$total, [int]$n)
    return $total + (Get-Bumped $n)
}

function Invoke-Run {
    Add-Total 0 2
}
