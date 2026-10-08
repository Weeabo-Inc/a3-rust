<#
.SYNOPSIS
  Start, stop or check the shared GhidraMCP headless server (bethington/ghidra-mcp).

.DESCRIPTION
  One long-running JVM opens the shared Ghidra project (arma3_x64.exe, fully analysed) and serves
  the GhidraMCP HTTP API on http://127.0.0.1:8089. Every agent queries it through tools/re/re.py
  or plain HTTP; the MCP bridge in .mcp.json talks to the same server. Only this process may have
  the project open: stop it before running analyzeHeadless against the project.

  Paths (override with environment variables):
    A3_RE_WORK        P:\a3-rust\.work                    scratch root (logs, pid file)
    A3_GHIDRA_HOME    P:\a3-rust\.resources\ghidra_12.1.4_PUBLIC
    A3_GHIDRA_PROJECT P:\a3-ghidra\a3.gpr                 junction to .work\ghidra (Ghidra rejects
                                                          path elements starting with '.')
    A3_GHIDRA_MCP     P:\a3-rust\.work\tools\ghidra-mcp   clone + Maven headless build

.EXAMPLE
  powershell -File tools/re/ghidra-mcp-server.ps1 start
  powershell -File tools/re/ghidra-mcp-server.ps1 status
  powershell -File tools/re/ghidra-mcp-server.ps1 stop
#>
param(
    [Parameter(Position = 0)][ValidateSet('start', 'stop', 'status', 'restart')][string]$Action = 'status',
    [int]$Port = 8089,
    [string]$Program = 'arma3_x64.exe',
    [string]$MaxHeap = '12g'
)

$ErrorActionPreference = 'Stop'
function Env-Or($name, $default) { $v = [Environment]::GetEnvironmentVariable($name); if ($v) { $v } else { $default } }

$Work = Env-Or 'A3_RE_WORK' 'P:\a3-rust\.work'
$GhidraHome = Env-Or 'A3_GHIDRA_HOME' 'P:\a3-rust\.resources\ghidra_12.1.4_PUBLIC'
$Project = Env-Or 'A3_GHIDRA_PROJECT' 'P:\a3-ghidra\a3.gpr'
$McpHome = Env-Or 'A3_GHIDRA_MCP' (Join-Path $Work 'tools\ghidra-mcp')
$PidFile = Join-Path $Work 'ghidra-mcp-server.pid'
$LogOut = Join-Path $Work 'logs\ghidra-mcp-server.out.log'
$LogErr = Join-Path $Work 'logs\ghidra-mcp-server.err.log'
$ArgFile = Join-Path $Work 'ghidra-mcp-server.args'
$Url = "http://127.0.0.1:$Port"

function Get-ServerProcess {
    if (-not (Test-Path $PidFile)) { return $null }
    $id = [int](Get-Content $PidFile -Raw)
    Get-Process -Id $id -ErrorAction SilentlyContinue
}

function Test-Health {
    try { Invoke-RestMethod -Uri "$Url/check_connection" -TimeoutSec 5 } catch { $null }
}

function Start-Server {
    if (Get-ServerProcess) { Write-Output "already running (pid $(Get-Content $PidFile))"; return }
    $jar = Get-ChildItem (Join-Path $McpHome 'target') -Filter 'GhidraMCP-*.jar' | Select-Object -First 1
    if (-not $jar) { throw "GhidraMCP jar not found; build it: cd $McpHome; mvn clean package -P headless -DskipTests" }
    # Ghidra's own jars (not the copies in target\lib) so the runtime matches the installation.
    $jars = @($jar.FullName) + (Get-ChildItem (Join-Path $GhidraHome 'Ghidra') -Recurse -Filter *.jar |
            Where-Object { $_.FullName -match '\\lib\\[^\\]+\.jar$' -and $_.FullName -notmatch '\\Extensions\\' } |
            ForEach-Object FullName)
    $cp = ($jars | ForEach-Object { $_ -replace '\\', '/' }) -join ';'
    # Java @argfile: forward slashes avoid backslash-escape handling inside quotes.
    @(
        "-Xmx$MaxHeap", '-XX:+UseG1GC', "-Dghidra.home=$($GhidraHome -replace '\\','/')",
        '-cp', "`"$cp`"", 'com.xebyte.headless.GhidraMCPHeadlessServer',
        '--bind', '127.0.0.1', '--port', "$Port",
        '--project', "`"$($Project -replace '\\','/')`"", '--program', "`"$Program`""
    ) | Set-Content -Encoding ascii $ArgFile
    New-Item -ItemType Directory -Force (Split-Path $LogOut) | Out-Null
    $env:GHIDRA_MCP_ALLOW_SCRIPTS = '1'   # enables /run_script_inline (loopback only)
    $p = Start-Process -FilePath 'java' -ArgumentList "@$ArgFile" -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $LogOut -RedirectStandardError $LogErr
    Set-Content -Path $PidFile -Value $p.Id
    Write-Output "started pid $($p.Id); waiting for $Url ..."
    for ($i = 0; $i -lt 450; $i++) {
        Start-Sleep -Seconds 2
        if ($p.HasExited) { throw "server exited (code $($p.ExitCode)); see $LogErr" }
        $h = Test-Health
        if ($h) { Write-Output ($h | ConvertTo-Json -Compress); return }
    }
    throw "server did not answer within 15 minutes; see $LogOut"
}

function Stop-Server {
    $p = Get-ServerProcess
    if (-not $p) { Write-Output 'not running'; Remove-Item $PidFile -ErrorAction SilentlyContinue; return }
    # Save every open program first so renames/comments persist in the project.
    try { Invoke-RestMethod -Uri "$Url/save_all_programs" -TimeoutSec 300 | Out-Null } catch { }
    Stop-Process -Id $p.Id
    $p.WaitForExit(60000) | Out-Null
    Remove-Item $PidFile -ErrorAction SilentlyContinue
    Write-Output "stopped pid $($p.Id)"
}

switch ($Action) {
    'start' { Start-Server }
    'stop' { Stop-Server }
    'restart' { Stop-Server; Start-Server }
    'status' {
        $p = Get-ServerProcess
        $h = Test-Health
        if ($p -and $h) { Write-Output "running pid $($p.Id) at $Url"; Write-Output ($h | ConvertTo-Json -Compress) }
        elseif ($h) { Write-Output "a server answers at $Url (not started by this script)"; Write-Output ($h | ConvertTo-Json -Compress) }
        elseif ($p) { Write-Output "pid $($p.Id) alive but $Url not answering (starting up?)" }
        else { Write-Output 'not running'; exit 1 }
    }
}
