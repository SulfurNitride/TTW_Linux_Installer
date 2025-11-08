using TtwInstaller.Models;
using TtwInstaller.Utils;

namespace TtwInstaller.Services;

/// <summary>
/// Validates game installations and prerequisites before starting TTW installation
/// </summary>
public class ValidationService
{
    private readonly InstallConfig _config;
    private readonly LocationResolver _locationResolver;

    public ValidationService(InstallConfig config, LocationResolver locationResolver)
    {
        _config = config;
        _locationResolver = locationResolver;
    }

    /// <summary>
    /// Run all validation checks from the manifest
    /// </summary>
    public bool RunValidationChecks(List<Check>? checks)
    {
        if (checks == null || checks.Count == 0)
        {
            Console.WriteLine("No validation checks found in manifest.");
            return true;
        }

        Console.WriteLine($"\n=== Running Pre-Flight Validation ({checks.Count} checks) ===\n");

        bool allPassed = true;
        int passedCount = 0;

        foreach (var check in checks)
        {
            bool checkPassed = RunSingleCheck(check);

            if (checkPassed)
            {
                passedCount++;
            }
            else
            {
                allPassed = false;
            }
        }

        Console.WriteLine($"\n=== Validation Complete: {passedCount}/{checks.Count} checks passed ===\n");

        return allPassed;
    }

    /// <summary>
    /// Run validation checks and return detailed results
    /// </summary>
    public (bool Success, string ErrorDetails) RunValidationChecksWithDetails(List<Check>? checks)
    {
        if (checks == null || checks.Count == 0)
        {
            Console.WriteLine("No validation checks found in manifest.");
            return (true, string.Empty);
        }

        Console.WriteLine($"\n=== Running Pre-Flight Validation ({checks.Count} checks) ===\n");

        var errors = new List<string>();
        int passedCount = 0;

        foreach (var check in checks)
        {
            var (passed, errorMsg) = RunSingleCheckWithDetails(check);

            if (passed)
            {
                passedCount++;
            }
            else if (!string.IsNullOrEmpty(errorMsg))
            {
                errors.Add(errorMsg);
            }
        }

        Console.WriteLine($"\n=== Validation Complete: {passedCount}/{checks.Count} checks passed ===\n");

        if (errors.Count > 0)
        {
            return (false, string.Join("\n\n", errors));
        }

        return (true, string.Empty);
    }

    /// <summary>
    /// Run a single validation check and return detailed result
    /// </summary>
    private (bool Success, string ErrorMessage) RunSingleCheckWithDetails(Check check)
    {
        try
        {
            switch (check.Type)
            {
                case 0: // FileExists
                    return CheckFileExistsWithDetails(check);

                case 1: // FreeSize
                    return CheckFreeSizeWithDetails(check);

                case 2: // NoProgramFiles
                    return CheckNoProgramFilesWithDetails(check);

                default:
                    Console.WriteLine($"⚠️  Unknown check type: {check.Type}");
                    return (true, string.Empty); // Don't fail on unknown check types
            }
        }
        catch (Exception ex)
        {
            string error = $"Error running check: {ex.Message}";
            Console.WriteLine($"⚠️  {error}");
            return (false, error);
        }
    }

    /// <summary>
    /// Run a single validation check
    /// </summary>
    private bool RunSingleCheck(Check check)
    {
        try
        {
            switch (check.Type)
            {
                case 0: // FileExists
                    return CheckFileExists(check);

                case 1: // FreeSize
                    return CheckFreeSize(check);

                case 2: // NoProgramFiles
                    return CheckNoProgramFiles(check);

                default:
                    Console.WriteLine($"⚠️  Unknown check type: {check.Type}");
                    return true; // Don't fail on unknown check types
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️  Error running check: {ex.Message}");
            return false;
        }
    }

    /// <summary>
    /// Check if a file exists and optionally verify its checksum
    /// </summary>
    private bool CheckFileExists(Check check)
    {
        if (string.IsNullOrEmpty(check.File))
        {
            // Empty file check - just verify location exists
            return true;
        }

        // Resolve the file path
        string basePath = _locationResolver.GetDirectoryPath(check.Loc);
        string filePath = Path.Combine(basePath, check.File);

        bool fileExists = File.Exists(filePath);

        // Apply inversion logic
        bool checkResult = check.Inverted ? !fileExists : fileExists;

        if (!checkResult)
        {
            Console.WriteLine($"❌ File Check Failed: {check.File}");
            if (!string.IsNullOrEmpty(check.CustomMessage))
            {
                Console.WriteLine($"   {check.CustomMessage}\n");
            }
            return false;
        }

        // If file should exist, optionally verify checksum
        if (!check.Inverted && fileExists && !string.IsNullOrEmpty(check.Checksums))
        {
            var expectedChecksums = check.Checksums
                .Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries)
                .Select(s => s.Trim().ToUpperInvariant())
                .Where(s => !string.IsNullOrEmpty(s))
                .ToList();

            Console.Write($"🔍 Verifying {check.File}... ");

            string actualChecksum = ChecksumHelper.CalculateSHA1(filePath);

            bool checksumMatches = expectedChecksums.Any(expected =>
                string.Equals(expected, actualChecksum, StringComparison.OrdinalIgnoreCase));

            if (!checksumMatches)
            {
                Console.WriteLine($"❌ FAILED");
                Console.WriteLine($"   Expected one of:");
                foreach (var expected in expectedChecksums.Take(3))
                {
                    Console.WriteLine($"     {expected}");
                }
                if (expectedChecksums.Count > 3)
                {
                    Console.WriteLine($"     ... and {expectedChecksums.Count - 3} more");
                }
                Console.WriteLine($"   Actual: {actualChecksum}");

                if (!string.IsNullOrEmpty(check.CustomMessage))
                {
                    Console.WriteLine($"\n   {check.CustomMessage}\n");
                }

                return false;
            }

            Console.WriteLine($"✅ OK ({actualChecksum.Substring(0, 8)}...)");
        }
        else if (!check.Inverted && fileExists)
        {
            Console.WriteLine($"✅ File exists: {check.File}");
        }
        else if (check.Inverted && !fileExists)
        {
            Console.WriteLine($"✅ File correctly absent: {check.File}");
        }

        return true;
    }

    /// <summary>
    /// Check if there's enough free space in the destination
    /// </summary>
    private bool CheckFreeSize(Check check)
    {
        // Disk space detection disabled - users are informed to have 20GB via startup dialog
        Console.WriteLine($"💾 Disk space check skipped (ensure 20GB+ available)");
        return true;
    }

    /// <summary>
    /// Check if a path is NOT in Program Files
    /// </summary>
    private bool CheckNoProgramFiles(Check check)
    {
        string path = _locationResolver.GetDirectoryPath(check.Loc);

        Console.Write($"📁 Checking installation path... ");

        // Check if path contains "Program Files" or "Program Files (x86)"
        bool isInProgramFiles = path.Contains("Program Files", StringComparison.OrdinalIgnoreCase);

        if (check.Inverted)
        {
            isInProgramFiles = !isInProgramFiles;
        }

        if (isInProgramFiles)
        {
            Console.WriteLine($"❌ INVALID");
            Console.WriteLine($"   Path: {path}");

            if (!string.IsNullOrEmpty(check.CustomMessage))
            {
                Console.WriteLine($"\n   {check.CustomMessage}\n");
            }

            return false;
        }

        Console.WriteLine($"✅ OK");
        return true;
    }

    // ===== Detailed versions that return error messages =====

    private (bool Success, string ErrorMessage) CheckFileExistsWithDetails(Check check)
    {
        if (string.IsNullOrEmpty(check.File))
        {
            return (true, string.Empty);
        }

        string basePath = _locationResolver.GetDirectoryPath(check.Loc);
        string filePath = Path.Combine(basePath, check.File);
        bool fileExists = File.Exists(filePath);
        bool checkResult = check.Inverted ? !fileExists : fileExists;

        if (!checkResult)
        {
            Console.WriteLine($"❌ File Check Failed: {check.File}");
            string error = $"❌ File Check Failed: {check.File}\n   Path: {filePath}";
            if (!string.IsNullOrEmpty(check.CustomMessage))
            {
                error += $"\n   {check.CustomMessage}";
                Console.WriteLine($"   {check.CustomMessage}\n");
            }
            return (false, error);
        }

        if (!check.Inverted && fileExists && !string.IsNullOrEmpty(check.Checksums))
        {
            var expectedChecksums = check.Checksums
                .Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries)
                .Select(s => s.Trim().ToUpperInvariant())
                .Where(s => !string.IsNullOrEmpty(s))
                .ToList();

            Console.Write($"🔍 Verifying {check.File}... ");
            string actualChecksum = ChecksumHelper.CalculateSHA1(filePath);

            bool checksumMatches = expectedChecksums.Any(expected =>
                string.Equals(expected, actualChecksum, StringComparison.OrdinalIgnoreCase));

            if (!checksumMatches)
            {
                Console.WriteLine($"❌ FAILED");
                string error = $"❌ Checksum Verification Failed: {check.File}\n" +
                              $"   Expected one of: {string.Join(", ", expectedChecksums.Take(2))}\n" +
                              $"   Actual: {actualChecksum}";

                if (!string.IsNullOrEmpty(check.CustomMessage))
                {
                    error += $"\n   {check.CustomMessage}";
                }

                Console.WriteLine($"   Expected one of:");
                foreach (var expected in expectedChecksums.Take(3))
                {
                    Console.WriteLine($"     {expected}");
                }
                if (expectedChecksums.Count > 3)
                {
                    Console.WriteLine($"     ... and {expectedChecksums.Count - 3} more");
                }
                Console.WriteLine($"   Actual: {actualChecksum}");

                if (!string.IsNullOrEmpty(check.CustomMessage))
                {
                    Console.WriteLine($"\n   {check.CustomMessage}\n");
                }

                return (false, error);
            }

            Console.WriteLine($"✅ OK ({actualChecksum.Substring(0, 8)}...)");
        }
        else if (!check.Inverted && fileExists)
        {
            Console.WriteLine($"✅ File exists: {check.File}");
        }
        else if (check.Inverted && !fileExists)
        {
            Console.WriteLine($"✅ File correctly absent: {check.File}");
        }

        return (true, string.Empty);
    }

    private (bool Success, string ErrorMessage) CheckFreeSizeWithDetails(Check check)
    {
        // Disk space detection disabled - users are informed to have 20GB via startup dialog
        Console.WriteLine($"💾 Disk space check skipped (ensure 20GB+ available)");
        return (true, string.Empty);
    }

    private (bool Success, string ErrorMessage) CheckNoProgramFilesWithDetails(Check check)
    {
        string path = _locationResolver.GetDirectoryPath(check.Loc);
        Console.Write($"📁 Checking installation path... ");

        bool isInProgramFiles = path.Contains("Program Files", StringComparison.OrdinalIgnoreCase);

        if (check.Inverted)
        {
            isInProgramFiles = !isInProgramFiles;
        }

        if (isInProgramFiles)
        {
            Console.WriteLine($"❌ INVALID");
            string error = $"❌ Invalid Installation Path\n" +
                          $"   Path: {path}\n" +
                          $"   Cannot install to Program Files directory";

            if (!string.IsNullOrEmpty(check.CustomMessage))
            {
                error += $"\n   {check.CustomMessage}";
            }

            Console.WriteLine($"   Path: {path}");

            if (!string.IsNullOrEmpty(check.CustomMessage))
            {
                Console.WriteLine($"\n   {check.CustomMessage}\n");
            }

            return (false, error);
        }

        Console.WriteLine($"✅ OK");
        return (true, string.Empty);
    }
}
