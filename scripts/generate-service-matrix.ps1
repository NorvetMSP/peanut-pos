# Generates the Service matrix table in docs/Architecture/Architecture_10_7_2025.md
# Sources docker-compose.yml, optional .env, and a topics map to avoid drift.
# Usage: pwsh ./scripts/generate-service-matrix.ps1 [-DocPath <path>] [-ComposePath <path>] [-TopicsPath <path>]

param(
    [string]$DocPath = "docs/Architecture/Architecture_10_7_2025.md",
    [string]$ComposePath = "docker-compose.yml",
    [string]$TopicsPath = "docs/topics-map.json"
)

function Get-EnvMap($path) {
    $map = @{}
    if (Test-Path $path) {
        Get-Content $path | ForEach-Object {
            if ($_ -match '^(?<k>[A-Za-z_][A-Za-z0-9_]*)=(?<v>.*)$') {
                $k = $matches.k; $v = $matches.v
                # Strip quotes
                if ($v.StartsWith('"') -and $v.EndsWith('"')) { $v = $v.Substring(1, $v.Length-2) }
                if ($v.StartsWith("'") -and $v.EndsWith("'")) { $v = $v.Substring(1, $v.Length-2) }
                $map[$k] = $v
            }
        }
    }
    return $map
}

function Get-ComposeYaml($path) {
    # Use PowerShell's ConvertFrom-Yaml if available (PowerShell 7+ / powershell-yaml module)
    $convertCmd = Get-Command ConvertFrom-Yaml -ErrorAction SilentlyContinue
    if ($convertCmd) {
        return (Get-Content $path -Raw) | ConvertFrom-Yaml
    }
    # Fallback: minimal parser that scans for service names and first published host port
    $lines = Get-Content $path
    $inServices = $false
    $currentService = $null
    $inPorts = $false
    $portsMap = @{}
    foreach ($line in $lines) {
        $trim = $line.Trim()
        if ($trim -match '^services:\s*$') { $inServices = $true; $currentService = $null; $inPorts = $false; continue }
        if (-not $inServices) { continue }
        # Service header: name:
        if ($line -match '^(\s{2,})([A-Za-z0-9_.\-]+):\s*$') {
            $currentService = $matches[2]
            $inPorts = $false
            continue
        }
        if (-not $currentService) { continue }
        # Detect end of current service block when dedenting to services level
        if ($line -match '^\s{0,1}[A-Za-z0-9_.\-]+:\s*$') { $currentService = $null; $inPorts = $false; continue }
        if ($trim -match '^ports:\s*$') { $inPorts = $true; continue }
        if ($inPorts) {
            # Short syntax: - "HOST:CONTAINER" or - HOST:CONTAINER
            if ($trim -match '^-\s*"?(?<host>\d+):(?<container>\d+)(/tcp)?"?\s*$') {
                $portsMap[$currentService] = [int]$matches['host']
                $inPorts = $false
                continue
            }
            # Long syntax block, look for published: <host>
            if ($trim -match '^published:\s*(?<pub>\d+)\s*$') {
                $portsMap[$currentService] = [int]$matches['pub']
                $inPorts = $false
                continue
            }
        }
    }
    return $portsMap
}

function Get-TopicsMap($path) {
    if (Test-Path $path) {
        return Get-Content $path -Raw | ConvertFrom-Json
    }
    return @{}
}

$envMap = Get-EnvMap ".env"
$compose = Get-ComposeYaml $ComposePath
$topics = Get-TopicsMap $TopicsPath

# Map of known services to folders and default health/metrics
$serviceMeta = @{
    "auth-service" = @{ Folder = "services/auth-service"; Health = "/healthz"; Metrics = "/metrics" };
    "product-service" = @{ Folder = "services/product-service"; Health = "/healthz"; Metrics = "/internal/metrics" };
    "inventory-service" = @{ Folder = "services/inventory-service"; Health = "/healthz"; Metrics = "/metrics" };
    "order-service" = @{ Folder = "services/order-service"; Health = "/healthz"; Metrics = "/metrics" };
    "payment-service" = @{ Folder = "services/payment-service"; Health = "/healthz"; Metrics = "/metrics" };
    "integration-gateway" = @{ Folder = "services/integration-gateway"; Health = "/healthz"; Metrics = "/metrics" };
    "customer-service" = @{ Folder = "services/customer-service"; Health = "/healthz"; Metrics = "/internal/metrics" };
    "loyalty-service" = @{ Folder = "services/loyalty-service"; Health = "/healthz"; Metrics = "/metrics" };
    "analytics-service" = @{ Folder = "services/analytics-service"; Health = "/healthz"; Metrics = "/metrics" };
}

# Try to determine ports from compose, falling back to typical defaults
function Get-ServicePort($compose, [string]$svcKey, $envMap) {
    # If compose is a simple hashtable of ports (fallback parser)
    if ($compose -is [hashtable]) {
        if ($compose.ContainsKey($svcKey)) { return [int]$compose[$svcKey] }
    }
    # Find service by exact key or fuzzy match
    $svc = $null
    if ($compose.services) {
        # Exact key (handles dashed names via property enumeration)
        $prop = $compose.services.PSObject.Properties | Where-Object { $_.Name -eq $svcKey } | Select-Object -First 1
        if ($prop) { $svc = $prop.Value }
        if (-not $svc) {
            # try to find by contains
            $prop = $compose.services.PSObject.Properties | Where-Object { $_.Name -like "*$svcKey*" } | Select-Object -First 1
            if ($prop) { $svc = $prop.Value }
        }
    }
    if ($svc -and $svc.ports) {
        foreach ($p in $svc.ports) {
            if ($p -is [string]) {
                $m = [regex]::Match($p, '^(?<host>\d+):(\d+)(/tcp)?$')
                if ($m.Success) { return [int]$m.Groups['host'].Value }
            } elseif ($p -is [hashtable] -or $p -is [psobject]) {
                $published = $null
                if ($p.published) { $published = $p.published }
                elseif ($p.PSObject -and ($pp = $p.PSObject.Properties | Where-Object { $_.Name -eq 'published' } | Select-Object -First 1)) { $published = $pp.Value }
                if ($published) { return [int]$published }
            }
        }
    }
    # fallback to env var like SERVICE_PORT or default
    $envKey = ($svcKey.ToUpper() + "_PORT")
    if ($envMap.ContainsKey($envKey)) { return [int]$envMap[$envKey] }
    switch ($svcKey) {
        "product-service" { return 8081 }
        "analytics-service" { return 8082 }
        "integration-gateway" { return 8083 }
        "order-service" { return 8084 }
        "auth-service" { return 8085 }
        "payment-service" { return 8086 }
        "inventory-service" { return 8087 }
        "loyalty-service" { return 8088 }
        "customer-service" { return 8089 }
        default { return $null }
    }
}

$rows = @()
foreach ($key in $serviceMeta.Keys) {
    $meta = $serviceMeta[$key]
    $port = Get-ServicePort $compose $key $envMap
    $folder = $meta.Folder
    $health = $meta.Health
    $metrics = $meta.Metrics

    # Topics map structure example:
    # {
    #   "auth-service": { "pub": ["security.mfa.activity", "audit.events"], "con": [] },
    #   "order-service": { "pub": ["order.completed", "order.voided"], "con": ["payment.completed", "payment.failed"] }
    # }
    # Retrieve topics by property name (works with dashed keys)
    $t = $null
    if ($topics) {
        $tProp = $topics.PSObject.Properties | Where-Object { $_.Name -eq $key } | Select-Object -First 1
        if ($tProp) { $t = $tProp.Value }
    }
    $pub = @()
    $con = @()
    if ($t) {
        if ($t.pub) { $pub = $t.pub }
        if ($t.con) { $con = $t.con }
    }
    $pubText = $null
    $conText = $null
    if ($pub -and $pub.Count -gt 0) {
        $pubLinks = @()
        foreach ($p in $pub) {
            $slug = ($p -replace '[^A-Za-z0-9_\-\.]','') -replace '\.',''
            $slug = $slug.ToLower()
            $pubLinks += "[$p](#$slug)"
        }
        $pubText = "Pub: " + ($pubLinks -join "; ")
    }
    if ($con -and $con.Count -gt 0) {
        $conLinks = @()
        foreach ($c in $con) {
            $slug = ($c -replace '[^A-Za-z0-9_\-\.]','') -replace '\.',''
            $slug = $slug.ToLower()
            $conLinks += "[$c](#$slug)"
        }
        $conText = "Con: " + ($conLinks -join ", ")
    }
    $topicsText = '—'
    if ($pubText -and $conText) { $topicsText = ($pubText + ' ; ' + $conText) }
    elseif ($pubText) { $topicsText = $pubText }
    elseif ($conText) { $topicsText = $conText }

    $rows += [PSCustomObject]@{
        Service = $key
        Folder = $folder
        Port = $port
        Health = $health
        Metrics = $metrics
        Topics = $topicsText
    }
}

# Render markdown table
$table = @()
$table += '| Service | Folder | Port (dev) | Health | Metrics | Primary topics |'
$table += '|---|---|---:|---|---|---|'
${bt} = [char]96
${displayNames} = @{
    'auth-service' = 'Auth'
    'product-service' = 'Product'
    'inventory-service' = 'Inventory'
    'order-service' = 'Order'
    'payment-service' = 'Payment'
    'integration-gateway' = 'Integration Gateway'
    'customer-service' = 'Customer'
    'loyalty-service' = 'Loyalty'
    'analytics-service' = 'Analytics'
}
foreach ($r in $rows) {
    $svcName = if ($displayNames.ContainsKey($r.Service)) { $displayNames[$r.Service] } else { $r.Service }
    $row = ('| {0} | {1}{2}{1} | {3} | {1}{4}{1} | {1}{5}{1} | {6} |' -f $svcName, $bt, $r.Folder, $r.Port, $r.Health, $r.Metrics, $r.Topics)
    $table += $row
}

# Update the doc between markers without typed delegates for PS 5.1 compatibility
$doc = Get-Content $DocPath -Raw
$begin = '<!-- service-matrix:begin -->'
$end = '<!-- service-matrix:end -->'
$nl = [Environment]::NewLine
$replacement = $begin + $nl + ($table -join $nl) + $nl + $end
$startIdx = $doc.IndexOf($begin)
if ($startIdx -ge 0) {
    $endIdx = $doc.IndexOf($end, $startIdx)
    if ($endIdx -ge 0) {
        $prefix = $doc.Substring(0, $startIdx)
        $suffix = $doc.Substring($endIdx + $end.Length)
        $newDoc = $prefix + $replacement + $suffix
    } else {
        $newDoc = $doc + $nl + $nl + $replacement
    }
} else {
    $newDoc = $doc + $nl + $nl + $replacement
}
$newDoc | Set-Content $DocPath -NoNewline
Write-Host ('Service matrix updated in {0}' -f $DocPath)
