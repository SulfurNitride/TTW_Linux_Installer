using TtwInstaller.Models;

namespace TtwInstaller.Services;

/// <summary>
/// Executes post-installation commands (file renaming, cleanup, etc.)
/// </summary>
public class PostCommandRunner
{
    private readonly string _destinationPath;

    public PostCommandRunner(string destinationPath)
    {
        _destinationPath = destinationPath;
    }

    /// <summary>
    /// Execute all post-installation commands
    /// </summary>
    public int RunPostCommands(List<PostCommand>? commands)
    {
        if (commands == null || commands.Count == 0)
        {
            return 0;
        }

        Console.WriteLine($"\n=== Running Post-Installation Commands ({commands.Count} commands) ===\n");

        int successCount = 0;
        int failCount = 0;

        foreach (var command in commands)
        {
            if (string.IsNullOrWhiteSpace(command.Value))
                continue;

            try
            {
                // Parse and execute command (convert Windows cmd.exe commands to cross-platform)
                if (ExecuteCommand(command.Value))
                {
                    successCount++;
                }
                else
                {
                    failCount++;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"  ⚠️  Command failed: {ex.Message}");
                failCount++;
            }
        }

        Console.WriteLine($"\nPost-commands: {successCount}/{commands.Count} succeeded, {failCount} failed\n");
        return failCount;
    }

    /// <summary>
    /// Execute a single command (translates Windows commands to cross-platform)
    /// </summary>
    private bool ExecuteCommand(string commandValue)
    {
        // Replace %DESTINATION% variable
        var command = commandValue.Replace("%DESTINATION%", _destinationPath);

        // Parse Windows cmd.exe commands
        if (command.Contains("cmd.exe") && command.Contains("/C"))
        {
            // Extract the actual command after "/C"
            var parts = command.Split(new[] { "/C" }, StringSplitOptions.None);
            if (parts.Length < 2)
                return false;

            var actualCommand = parts[1].Trim();

            // Handle "del" command (delete file)
            if (actualCommand.StartsWith("del ", StringComparison.OrdinalIgnoreCase))
            {
                var filePath = ExtractPath(actualCommand.Substring(4));
                if (!string.IsNullOrEmpty(filePath))
                {
                    if (File.Exists(filePath))
                    {
                        File.Delete(filePath);
                        Console.WriteLine($"  Deleted: {Path.GetFileName(filePath)}");
                    }
                    return true;
                }
            }
            // Handle "ren" command (rename file)
            else if (actualCommand.StartsWith("ren ", StringComparison.OrdinalIgnoreCase))
            {
                var args = actualCommand.Substring(4).Trim();
                var pathParts = SplitRenamePaths(args);

                if (pathParts.oldPath != null && pathParts.newPath != null)
                {
                    var oldPath = ExtractPath(pathParts.oldPath);
                    var newFileName = ExtractPath(pathParts.newPath);

                    if (!string.IsNullOrEmpty(oldPath) && !string.IsNullOrEmpty(newFileName))
                    {
                        var newPath = Path.Combine(Path.GetDirectoryName(oldPath) ?? _destinationPath, newFileName);

                        if (File.Exists(oldPath))
                        {
                            File.Move(oldPath, newPath, overwrite: true);
                            Console.WriteLine($"  Renamed: {Path.GetFileName(oldPath)} → {Path.GetFileName(newPath)}");
                        }
                        return true;
                    }
                }
            }
        }

        return false;
    }

    /// <summary>
    /// Extract file path from quoted or unquoted string
    /// </summary>
    private string ExtractPath(string pathString)
    {
        var path = pathString.Trim().Trim('"');
        // Normalize path separators
        return path.Replace('\\', Path.DirectorySeparatorChar);
    }

    /// <summary>
    /// Split rename command arguments into old and new paths
    /// </summary>
    private (string? oldPath, string? newPath) SplitRenamePaths(string args)
    {
        // Handle quoted paths: "old path" "new path"
        var parts = args.Split('"', StringSplitOptions.RemoveEmptyEntries);

        if (parts.Length >= 2)
        {
            // Find non-whitespace parts
            var nonEmpty = parts.Where(p => !string.IsNullOrWhiteSpace(p)).ToArray();
            if (nonEmpty.Length >= 2)
            {
                return (nonEmpty[0].Trim(), nonEmpty[1].Trim());
            }
        }

        return (null, null);
    }
}
