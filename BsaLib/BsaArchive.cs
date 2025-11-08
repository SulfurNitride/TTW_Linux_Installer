using System;
using System.IO;

namespace BsaLib;

/// <summary>
/// High-level C# wrapper for BSA archive creation
/// </summary>
public class BsaArchive : IDisposable
{
    private IntPtr _handle;
    private bool _disposed;

    public BsaArchive()
    {
        _handle = BsaInterop.bsa_create();
        if (_handle == IntPtr.Zero)
        {
            throw new InvalidOperationException("Failed to create BSA archive");
        }
    }

    /// <summary>
    /// Set archive flags (compression, embedded names, etc.)
    /// </summary>
    public void SetArchiveFlags(BsaInterop.ArchiveFlags flags)
    {
        ThrowIfDisposed();
        BsaInterop.bsa_set_archive_flags(_handle, flags);
    }

    /// <summary>
    /// Set archive types (meshes, textures, sounds, etc.)
    /// </summary>
    public void SetArchiveTypes(BsaInterop.ArchiveTypes types)
    {
        ThrowIfDisposed();
        BsaInterop.bsa_set_archive_types(_handle, types);
    }

    /// <summary>
    /// Add a file to the archive
    /// </summary>
    /// <param name="dirPath">Directory path within archive (e.g., "textures")</param>
    /// <param name="fileName">File name (e.g., "test.dds")</param>
    /// <param name="data">File data bytes</param>
    public void AddFile(string dirPath, string fileName, byte[] data)
    {
        ThrowIfDisposed();

        if (string.IsNullOrWhiteSpace(dirPath))
            throw new ArgumentException("Directory path cannot be empty", nameof(dirPath));

        if (string.IsNullOrWhiteSpace(fileName))
            throw new ArgumentException("File name cannot be empty", nameof(fileName));

        if (data == null || data.Length == 0)
            throw new ArgumentException("Data cannot be empty", nameof(data));

        int result = BsaInterop.bsa_add_file(_handle, dirPath, fileName, data, (nuint)data.Length);

        if (result != 0)
        {
            string? error = BsaInterop.bsa_get_last_error();
            throw new InvalidOperationException($"Failed to add file: {error ?? "Unknown error"}");
        }
    }

    /// <summary>
    /// Add a file from disk to the archive
    /// </summary>
    /// <param name="dirPath">Directory path within archive</param>
    /// <param name="fileName">File name within archive</param>
    /// <param name="filePath">Path to file on disk</param>
    public void AddFileFromDisk(string dirPath, string fileName, string filePath)
    {
        byte[] data = File.ReadAllBytes(filePath);
        AddFile(dirPath, fileName, data);
    }

    /// <summary>
    /// Write the archive to disk
    /// </summary>
    /// <param name="outputPath">Path to output BSA file</param>
    /// <param name="version">BSA version (use BsaInterop.BSA_VERSION_*)</param>
    public void Write(string outputPath, uint version)
    {
        ThrowIfDisposed();

        if (string.IsNullOrWhiteSpace(outputPath))
            throw new ArgumentException("Output path cannot be empty", nameof(outputPath));

        int result = BsaInterop.bsa_write(_handle, outputPath, version);

        if (result != 0)
        {
            string? error = BsaInterop.bsa_get_last_error();
            throw new InvalidOperationException($"Failed to write archive: {error ?? "Unknown error"}");
        }
    }

    private void ThrowIfDisposed()
    {
        if (_disposed)
            throw new ObjectDisposedException(nameof(BsaArchive));
    }

    public void Dispose()
    {
        if (!_disposed)
        {
            if (_handle != IntPtr.Zero)
            {
                BsaInterop.bsa_free(_handle);
                _handle = IntPtr.Zero;
            }
            _disposed = true;
        }
        GC.SuppressFinalize(this);
    }

    ~BsaArchive()
    {
        Dispose();
    }
}
