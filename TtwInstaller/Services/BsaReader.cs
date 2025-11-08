using System.Runtime.InteropServices;
using System.IO.Compression;
using BsaLib;

namespace TtwInstaller.Services;

/// <summary>
/// Reads files from BSA archives using native bsa library
/// Caches BSA handles for improved performance
/// </summary>
public class BsaReader : IDisposable
{
    private bool _disposed;
    private readonly Dictionary<string, IntPtr> _cachedHandles = new();
    private readonly object _cacheLock = new();

    /// <summary>
    /// Get or open a cached BSA handle (thread-safe)
    /// </summary>
    private IntPtr GetCachedHandle(string bsaPath)
    {
        // Normalize path for cache key
        var normalizedPath = Path.GetFullPath(bsaPath);

        // Thread-safe cache access
        lock (_cacheLock)
        {
            // Check cache first
            if (_cachedHandles.TryGetValue(normalizedPath, out var cachedHandle))
            {
                return cachedHandle;
            }

            // Open new BSA and cache it
            if (!File.Exists(bsaPath))
            {
                throw new FileNotFoundException($"BSA file not found: {bsaPath}");
            }

            var handle = BsaInterop.bsa_open_archive(bsaPath);
            if (handle == IntPtr.Zero)
            {
                var error = BsaInterop.bsa_get_last_error();
                throw new InvalidOperationException($"Failed to open BSA: {error}");
            }

            _cachedHandles[normalizedPath] = handle;
            return handle;
        }
    }

    /// <summary>
    /// Extract a file from a BSA archive (uses cached handles for performance)
    /// </summary>
    public byte[]? ExtractFile(string bsaPath, string filePath)
    {
        try
        {
            // Get cached handle (or open and cache if not already open)
            IntPtr handle = GetCachedHandle(bsaPath);

            // Extract file
            int result = BsaInterop.bsa_extract_file(
                handle,
                filePath,
                out IntPtr dataPtr,
                out nuint dataSize);

            if (result != 0)
            {
                return null;
            }

            // Copy data to managed array
            byte[] data = new byte[dataSize];
            Marshal.Copy(dataPtr, data, 0, (int)dataSize);

            // Free native memory
            BsaInterop.bsa_free_data(dataPtr);

            // Check for zlib compression (magic bytes 0x78 0x9c for default compression)
            // NIF files often have internal zlib compression separate from BSA compression
            if (data.Length >= 2 && data[0] == 0x78 && data[1] == 0x9C)
            {
                try
                {
                    // Decompress using zlib (skip 2-byte zlib header, use DeflateStream)
                    using var compressedStream = new MemoryStream(data, 2, data.Length - 2);
                    using var deflateStream = new DeflateStream(compressedStream, CompressionMode.Decompress);
                    using var decompressedStream = new MemoryStream();

                    // Use a timeout to prevent hanging on corrupted data
                    var decompressTask = Task.Run(() => deflateStream.CopyTo(decompressedStream));
                    if (!decompressTask.Wait(TimeSpan.FromSeconds(30)))
                    {
                        Console.WriteLine($"    Warning: zlib decompression timed out for {filePath}");
                        return data; // Return original compressed data
                    }

                    data = decompressedStream.ToArray();
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"    Warning: zlib decompression failed for {filePath}: {ex.Message}");
                    // Return original data if decompression fails
                }
            }

            return data;
        }
        catch (Exception ex)
        {
            Console.WriteLine($"    Error extracting from BSA: {ex.Message}");
            return null;
        }
    }

    /// <summary>
    /// Check if a file exists in a BSA archive (uses cached handle)
    /// </summary>
    public bool FileExists(string bsaPath, string filePath)
    {
        try
        {
            if (!File.Exists(bsaPath))
                return false;

            IntPtr handle = GetCachedHandle(bsaPath);
            int result = BsaInterop.bsa_file_exists(handle, filePath);
            return result == 1;
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// Get file count in archive (uses cached handle)
    /// </summary>
    public int GetFileCount(string bsaPath)
    {
        try
        {
            if (!File.Exists(bsaPath))
                return -1;

            IntPtr handle = GetCachedHandle(bsaPath);
            return BsaInterop.bsa_get_file_count(handle);
        }
        catch
        {
            return -1;
        }
    }

    public void Dispose()
    {
        if (!_disposed)
        {
            lock (_cacheLock)
            {
                // Close all cached BSA handles
                foreach (var handle in _cachedHandles.Values)
                {
                    if (handle != IntPtr.Zero)
                    {
                        BsaInterop.bsa_close_archive(handle);
                    }
                }
                _cachedHandles.Clear();

                _disposed = true;
            }
        }
        GC.SuppressFinalize(this);
    }
}
