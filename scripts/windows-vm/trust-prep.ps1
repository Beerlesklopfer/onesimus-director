# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Prepares the directories of the plugin trust test (ofd win_trust::tests::prepared_directories)
# below D:\odir-test\trust. Locale independent: only SIDs are used with icacls.
# Run on the Windows test host in an ELEVATED session (the owner "Administrators" can only be set from there).
$ErrorActionPreference = 'Continue'
$base = 'D:\odir-test\trust'
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
"user SID $me, elevated: $elevated"
if (Test-Path "$base\junction") { cmd /c rmdir "$base\junction" }
if (Test-Path $base) { icacls $base /grant "*${me}:(OI)(CI)F" /T /C /Q | Out-Null; Remove-Item -Recurse -Force $base }
function I { param([string[]]$a) $o = & icacls @a 2>&1; if ($LASTEXITCODE -ne 0) { "icacls $($a -join ' ') FAILED: $o" } }
# a locked directory: owner Administrators, no inheritance, SYSTEM and Administrators full, Users read
function Lock { param($d)
  I $d,'/setowner','*S-1-5-32-544','/Q'
  I $d,'/inheritance:r','/Q'
  I $d,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)RX','/Q'
}
New-Item -ItemType Directory -Force $base | Out-Null
Lock $base
function Own { param($p) I $p,'/setowner','*S-1-5-32-544','/Q' }
# good: a locked directory with a file in it
New-Item -ItemType Directory $base\good | Out-Null; [IO.File]::WriteAllText("$base\good\f.bin", 'bundle'); Own $base\good; Own $base\good\f.bin
# user_writable: owner Administrators, but the current user may modify
New-Item -ItemType Directory $base\user_writable | Out-Null; Own $base\user_writable; I $base\user_writable,'/grant',"*${me}:(OI)(CI)M",'/Q'
# user_owned: ACL fine, owner is the current user
New-Item -ItemType Directory $base\user_owned | Out-Null; I $base\user_owned,'/setowner',"*$me",'/Q'
# everyone_file.bin: Everyone may write the file
[IO.File]::WriteAllText("$base\everyone_file.bin", 'x'); Own $base\everyone_file.bin; I $base\everyone_file.bin,'/grant','*S-1-1-0:W','/Q'
# inherit_dir: Users modify for new children only (inherit only): the directory is fine, its child is not
New-Item -ItemType Directory $base\inherit_dir | Out-Null; Own $base\inherit_dir
I $base\inherit_dir,'/grant','*S-1-5-32-545:(OI)(CI)(IO)M','/Q'
[IO.File]::WriteAllText("$base\inherit_dir\child.bin", 'x'); Own $base\inherit_dir\child.bin
# deny_cancels: Users get modify and a deny entry for Users in front of it
New-Item -ItemType Directory $base\deny_cancels | Out-Null; Own $base\deny_cancels; I $base\deny_cancels,'/inheritance:r','/Q'
I $base\deny_cancels,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)M','/Q'
I $base\deny_cancels,'/deny','*S-1-5-32-545:(OI)(CI)M','/Q'
# deny_other_group: Users modify, a deny entry for another group (Guests) does not help
New-Item -ItemType Directory $base\deny_other_group | Out-Null; Own $base\deny_other_group; I $base\deny_other_group,'/inheritance:r','/Q'
I $base\deny_other_group,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)M','/Q'
I $base\deny_other_group,'/deny','*S-1-5-32-546:(OI)(CI)M','/Q'
# with_delete_child_above: Users may delete the entries of the directory above a locked sub directory
New-Item -ItemType Directory $base\with_delete_child_above | Out-Null; Own $base\with_delete_child_above
I $base\with_delete_child_above,'/grant','*S-1-5-32-545:(DC)','/Q'
New-Item -ItemType Directory $base\with_delete_child_above\sub | Out-Null; Own $base\with_delete_child_above\sub
I $base\with_delete_child_above\sub,'/inheritance:r','/Q'
I $base\with_delete_child_above\sub,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)RX','/Q'
# adds_above: Users may add files and sub directories above a locked sub directory (the Windows default at a drive root)
New-Item -ItemType Directory $base\adds_above | Out-Null; Own $base\adds_above
I $base\adds_above,'/grant','*S-1-5-32-545:(WD,AD)','/Q'
New-Item -ItemType Directory $base\adds_above\sub | Out-Null; Own $base\adds_above\sub
I $base\adds_above\sub,'/inheritance:r','/Q'
I $base\adds_above\sub,'/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)RX','/Q'
# junction: a junction to good
cmd /c mklink /J "$base\junction" "$base\good" | Out-Null
"prepared below $base"
