using System.Runtime.InteropServices;
using System.IO.Compression;
using BsaLib;

namespace TtwInstaller.Services;

/// <summary>
/// Reads files from BSA archives using native bsa library
/// Opens BSA, extracts file, and immediately closes - no caching
/// </summary>
public class BsaReader : IDisposable
{
    private bool _disposed;

    /// <summary>
    /// Extract a file from a BSA archive (opens, extracts, and closes immediately)
    /// </summary>
    public byte[]? ExtractFile(string bsaPath, string filePath)
    {
        IntPtr handle = IntPtr.Zero;
        try
        {
            if (!File.Exists(bsaPath))
            {
                throw new FileNotFoundException($"BSA file not found: {bsaPath}");
            }

            // Open BSA temporarily
            handle = BsaInterop.bsa_open_archive(bsaPath);
            if (handle == IntPtr.Zero)
            {
                var error = BsaInterop.bsa_get_last_error();
                throw new InvalidOperationException($"Failed to open BSA: {error}");
            }

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
        finally
        {
            // Always close the BSA immediately after extracting
            if (handle != IntPtr.Zero)
            {
                BsaInterop.bsa_close_archive(handle);
            }
        }
    }

    /// <summary>
    /// Check if a file exists in a BSA archive
    /// </summary>
    public bool FileExists(string bsaPath, string filePath)
    {
        IntPtr handle = IntPtr.Zero;
        try
        {
            if (!File.Exists(bsaPath))
                return false;

            handle = BsaInterop.bsa_open_archive(bsaPath);
            if (handle == IntPtr.Zero)
                return false;

            int result = BsaInterop.bsa_file_exists(handle, filePath);
            return result == 1;
        }
        catch
        {
            return false;
        }
        finally
        {
            if (handle != IntPtr.Zero)
                BsaInterop.bsa_close_archive(handle);
        }
    }

    /// <summary>
    /// Get file count in archive
    /// </summary>
    public int GetFileCount(string bsaPath)
    {
        IntPtr handle = IntPtr.Zero;
        try
        {
            if (!File.Exists(bsaPath))
                return -1;

            handle = BsaInterop.bsa_open_archive(bsaPath);
            if (handle == IntPtr.Zero)
                return -1;

            return BsaInterop.bsa_get_file_count(handle);
        }
        finally
        {
            if (handle != IntPtr.Zero)
                BsaInterop.bsa_close_archive(handle);
        }
    }

    public void Dispose()
    {
        if (!_disposed)
        {
            // No cached handles to clean up since we open/close immediately
            _disposed = true;
        }
        GC.SuppressFinalize(this);
    }
}
