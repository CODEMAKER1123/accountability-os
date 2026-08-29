param(
  [string]$KeyPath = (Join-Path ([Environment]::GetFolderPath("UserProfile")) ".tauri\accountability-os-updater.key"),
  [string]$CredentialTarget = "tauri-updater.accountability-os"
)

$ErrorActionPreference = "Stop"

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class AccountabilityCredentialReader
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct Credential
    {
        public uint Flags;
        public uint Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, uint type, uint flags, out IntPtr credentialPointer);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern void CredFree(IntPtr buffer);

    public static string ReadPassword(string target)
    {
        IntPtr credentialPointer;
        if (!CredRead(target, 1, 0, out credentialPointer))
        {
            throw new InvalidOperationException("Windows Credential Manager entry was not found: " + target);
        }

        try
        {
            Credential credential = Marshal.PtrToStructure<Credential>(credentialPointer);
            return Marshal.PtrToStringUni(
                credential.CredentialBlob,
                checked((int)credential.CredentialBlobSize / 2)
            );
        }
        finally
        {
            CredFree(credentialPointer);
        }
    }
}
"@

$resolvedKeyPath = [IO.Path]::GetFullPath($KeyPath)
if (-not [IO.File]::Exists($resolvedKeyPath)) {
  throw "Updater signing key was not found: $resolvedKeyPath"
}
$releaseConfigPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\src-tauri\tauri.release.conf.json"))
if (-not [IO.File]::Exists($releaseConfigPath)) {
  throw "Updater release configuration was not found: $releaseConfigPath"
}

$previousKey = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", "Process")
$previousPassword = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
$previousProcessPath = [Environment]::GetEnvironmentVariable("Path", "Process")
$rustBinPath = Join-Path ([Environment]::GetFolderPath("UserProfile")) ".cargo\bin"

try {
  $env:TAURI_SIGNING_PRIVATE_KEY = [IO.File]::ReadAllText($resolvedKeyPath)
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [AccountabilityCredentialReader]::ReadPassword($CredentialTarget)
  if ((Test-Path -LiteralPath (Join-Path $rustBinPath "cargo.exe")) -and
      -not (($env:Path -split ";") -contains $rustBinPath)) {
    $env:Path = "$rustBinPath;$env:Path"
  }

  Write-Host "Building signed Windows installer and updater artifacts..."
  & npm.cmd run tauri -- build --config $releaseConfigPath
  if ($LASTEXITCODE -ne 0) {
    throw "The signed Tauri build failed with exit code $LASTEXITCODE."
  }
}
finally {
  [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", $previousKey, "Process")
  [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $previousPassword, "Process")
  [Environment]::SetEnvironmentVariable("Path", $previousProcessPath, "Process")
}
