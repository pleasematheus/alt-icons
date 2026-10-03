# Embedded in the crate and run by system Windows PowerShell without a window.
# The caller defines ownerId, ownerStarted (UTC FILETIME), oldPath, and launchPath.
$ErrorActionPreference = 'Stop'

if ($ownerId -ne 0) {
    $owner = $null
    try {
        $owner = [System.Diagnostics.Process]::GetProcessById($ownerId)
        # Do not wait on an unrelated process if Windows has already reused the PID.
        if ($owner.StartTime.ToUniversalTime().ToFileTimeUtc() -eq $ownerStarted) {
            $owner.WaitForExit()
        }
    } catch {
        # The owner may have exited before the helper opened it.
    } finally {
        if ($null -ne $owner) { $owner.Dispose() }
    }
}

# Retry short-lived locks (for example an antivirus scan), but do not keep a
# helper alive forever for an inaccessible file. Startup cleanup remains a fallback.
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while ($true) {
    try {
        [System.IO.File]::Delete($oldPath)
        break
    } catch {
        # Module inspection can still report the original launch path after a
        # rename. Wait for instances at either path. This can also wait for an
        # instance using the new image, but never deletes an image still in use.
        $readers = 0
        foreach ($candidate in [System.Diagnostics.Process]::GetProcesses()) {
            try {
                $modulePath = $candidate.MainModule.FileName
                if ($modulePath -eq $oldPath -or $modulePath -eq $launchPath) {
                    $readers++
                    $candidate.WaitForExit()
                }
            } catch {
                # A process can exit during enumeration or deny module inspection.
            } finally {
                $candidate.Dispose()
            }
        }
        if ($readers -gt 0) {
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
        } elseif ([DateTime]::UtcNow -ge $deadline) {
            break
        }
        [System.Threading.Thread]::Sleep(200)
    }
}
