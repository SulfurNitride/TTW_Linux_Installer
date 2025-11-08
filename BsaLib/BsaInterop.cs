using System;
using System.Runtime.InteropServices;

namespace BsaLib;

/// <summary>
/// P/Invoke wrapper for the native BSA library
/// </summary>
public static class BsaInterop
{
    private const string LibraryName = "libbsa_capi.so";

    // Archive version constants
    public const uint BSA_VERSION_TES4 = 103;
    public const uint BSA_VERSION_FO3  = 104;
    public const uint BSA_VERSION_FNV  = 104;
    public const uint BSA_VERSION_TES5 = 104;
    public const uint BSA_VERSION_SSE  = 105;

    // Archive flags
    [Flags]
    public enum ArchiveFlags : uint
    {
        None                        = 0x00000000,
        DirectoryStrings            = 0x00000001,
        FileStrings                 = 0x00000002,
        Compressed                  = 0x00000004,
        RetainDirectoryNames        = 0x00000008,
        RetainFileNames             = 0x00000010,
        RetainFileNameOffsets       = 0x00000020,
        XboxArchive                 = 0x00000040,
        RetainStringsDuringStartup  = 0x00000080,
        EmbeddedFileNames           = 0x00000100,
        XmemCodec                   = 0x00000200,
    }

    // Archive types
    [Flags]
    public enum ArchiveTypes : ushort
    {
        None     = 0x0000,
        Meshes   = 0x0001,
        Textures = 0x0002,
        Menus    = 0x0004,
        Sounds   = 0x0008,
        Voices   = 0x0010,
        Shaders  = 0x0020,
        Trees    = 0x0040,
        Fonts    = 0x0080,
        Misc     = 0x0100,
    }

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr bsa_create();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void bsa_free(IntPtr archive);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void bsa_set_archive_flags(IntPtr archive, ArchiveFlags flags);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void bsa_set_archive_types(IntPtr archive, ArchiveTypes types);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern int bsa_add_file(
        IntPtr archive,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dirPath,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string fileName,
        byte[] data,
        nuint dataSize);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern int bsa_write(
        IntPtr archive,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string outputPath,
        uint version);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.LPUTF8Str)]
    public static extern string? bsa_get_last_error();

    // ========================================================================
    // BSA Reading Functions
    // ========================================================================

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr bsa_open_archive(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string archivePath);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void bsa_close_archive(IntPtr archive);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern int bsa_extract_file(
        IntPtr archive,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string filePath,
        out IntPtr outputData,
        out nuint outputSize);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void bsa_free_data(IntPtr data);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern int bsa_file_exists(
        IntPtr archive,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string filePath);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    public static extern int bsa_get_file_count(IntPtr archive);
}
