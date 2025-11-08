using System.Text;

namespace TtwInstaller.Services;

/// <summary>
/// Tracks errors, warnings, and issues during installation
/// </summary>
public class InstallationLogger
{
    private readonly List<string> _errors = new();
    private readonly List<string> _warnings = new();
    private readonly List<string> _missingFiles = new();
    private readonly object _lock = new();

    public void LogError(string message)
    {
        lock (_lock)
        {
            _errors.Add(message);
            Console.WriteLine($"❌ ERROR: {message}");
        }
    }

    public void LogWarning(string message)
    {
        lock (_lock)
        {
            _warnings.Add(message);
            Console.WriteLine($"⚠️  WARNING: {message}");
        }
    }

    public void LogMissingFile(string filePath, string reason = "")
    {
        lock (_lock)
        {
            string msg = string.IsNullOrEmpty(reason)
                ? filePath
                : $"{filePath} - {reason}";
            _missingFiles.Add(msg);
            Console.WriteLine($"⚠️  Missing file: {msg}");
        }
    }

    public bool HasErrors => _errors.Count > 0;
    public bool HasWarnings => _warnings.Count > 0;
    public bool HasIssues => HasErrors || HasWarnings || _missingFiles.Count > 0;

    public int ErrorCount => _errors.Count;
    public int WarningCount => _warnings.Count;
    public int MissingFileCount => _missingFiles.Count;

    /// <summary>
    /// Get a summary of all issues
    /// </summary>
    public string GetSummary()
    {
        if (!HasIssues)
            return "No issues detected during installation.";

        var sb = new StringBuilder();
        sb.AppendLine("=== Installation Issues Summary ===\n");

        if (_errors.Count > 0)
        {
            sb.AppendLine($"❌ ERRORS ({_errors.Count}):");
            foreach (var error in _errors)
            {
                sb.AppendLine($"   • {error}");
            }
            sb.AppendLine();
        }

        if (_warnings.Count > 0)
        {
            sb.AppendLine($"⚠️  WARNINGS ({_warnings.Count}):");
            foreach (var warning in _warnings)
            {
                sb.AppendLine($"   • {warning}");
            }
            sb.AppendLine();
        }

        if (_missingFiles.Count > 0)
        {
            sb.AppendLine($"📁 MISSING FILES ({_missingFiles.Count}):");
            int displayCount = Math.Min(_missingFiles.Count, 20);
            foreach (var file in _missingFiles.Take(displayCount))
            {
                sb.AppendLine($"   • {file}");
            }
            if (_missingFiles.Count > displayCount)
            {
                sb.AppendLine($"   ... and {_missingFiles.Count - displayCount} more");
            }
            sb.AppendLine();
        }

        return sb.ToString();
    }

    /// <summary>
    /// Write detailed log to file
    /// </summary>
    public void WriteLogFile(string outputPath)
    {
        try
        {
            var logPath = Path.Combine(outputPath, "ttw-installation.log");
            var sb = new StringBuilder();

            sb.AppendLine($"TTW Installation Log - {DateTime.Now}");
            sb.AppendLine(new string('=', 60));
            sb.AppendLine();

            sb.AppendLine($"Installation Directory: {outputPath}");
            sb.AppendLine($"Total Errors: {_errors.Count}");
            sb.AppendLine($"Total Warnings: {_warnings.Count}");
            sb.AppendLine($"Missing Files: {_missingFiles.Count}");
            sb.AppendLine();

            if (_errors.Count > 0)
            {
                sb.AppendLine("ERRORS:");
                sb.AppendLine(new string('-', 60));
                foreach (var error in _errors)
                {
                    sb.AppendLine($"  • {error}");
                }
                sb.AppendLine();
            }

            if (_warnings.Count > 0)
            {
                sb.AppendLine("WARNINGS:");
                sb.AppendLine(new string('-', 60));
                foreach (var warning in _warnings)
                {
                    sb.AppendLine($"  • {warning}");
                }
                sb.AppendLine();
            }

            if (_missingFiles.Count > 0)
            {
                sb.AppendLine("MISSING FILES:");
                sb.AppendLine(new string('-', 60));
                foreach (var file in _missingFiles)
                {
                    sb.AppendLine($"  • {file}");
                }
                sb.AppendLine();
            }

            File.WriteAllText(logPath, sb.ToString());
            Console.WriteLine($"\n📝 Detailed log written to: {logPath}");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️  Failed to write log file: {ex.Message}");
        }
    }
}
