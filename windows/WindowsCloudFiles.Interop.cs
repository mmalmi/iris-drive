using System;
using System.Runtime.InteropServices;

namespace IrisDrive.WindowsShell;

public static partial class WindowsCloudFiles
{
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    private delegate void CfCallback(IntPtr callbackInfo, IntPtr callbackParameters);

    [StructLayout(LayoutKind.Sequential)]
    private struct CfCallbackRegistration
    {
        public int Type;
        public IntPtr Callback;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfCallbackInfo
    {
        public uint StructSize;
        public long ConnectionKey;
        public IntPtr CallbackContext;
        public IntPtr VolumeGuidName;
        public IntPtr VolumeDosName;
        public uint VolumeSerialNumber;
        public long SyncRootFileId;
        public IntPtr SyncRootIdentity;
        public uint SyncRootIdentityLength;
        public long FileId;
        public long FileSize;
        public IntPtr FileIdentity;
        public uint FileIdentityLength;
        public IntPtr NormalizedPath;
        public long TransferKey;
        public byte PriorityHint;
        public IntPtr CorrelationVector;
        public IntPtr ProcessInfo;
        public long RequestKey;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationInfo
    {
        public uint StructSize;
        public int Type;
        public long ConnectionKey;
        public long TransferKey;
        public IntPtr CorrelationVector;
        public IntPtr SyncStatus;
        public long RequestKey;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationParametersTransferData
    {
        public uint ParamSize;
        private readonly uint padding;
        public CfOperationTransferData TransferData;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationParametersAckDelete
    {
        public uint ParamSize;
        private readonly uint padding;
        public CfOperationAckDelete AckDelete;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfCallbackParametersRename
    {
        public uint ParamSize;
        private readonly uint padding;
        public CfCallbackRename Rename;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationParametersAckRename
    {
        public uint ParamSize;
        private readonly uint padding;
        public CfOperationAckRename AckRename;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationTransferData
    {
        public int Flags;
        public int CompletionStatus;
        public IntPtr Buffer;
        public long Offset;
        public long Length;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationAckDelete
    {
        public int Flags;
        public int CompletionStatus;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfCallbackRename
    {
        public int Flags;
        public IntPtr TargetPath;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfOperationAckRename
    {
        public int Flags;
        public int CompletionStatus;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct FileBasicInfo
    {
        public long CreationTime;
        public long LastAccessTime;
        public long LastWriteTime;
        public long ChangeTime;
        public uint FileAttributes;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfFsMetadata
    {
        public FileBasicInfo BasicInfo;
        public long FileSize;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfPlaceholderCreateInfo
    {
        public IntPtr RelativeFileName;
        public CfFsMetadata FsMetadata;
        public IntPtr FileIdentity;
        public uint FileIdentityLength;
        public int Flags;
        public int Result;
        public long CreateUsn;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfHydrationPolicy
    {
        public ushort Primary;
        public ushort Modifier;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfPopulationPolicy
    {
        public ushort Primary;
        public ushort Modifier;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CfSyncPolicies
    {
        public uint StructSize;
        public CfHydrationPolicy Hydration;
        public CfPopulationPolicy Population;
        public uint InSync;
        public int HardLink;
        public int PlaceholderManagement;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct CfSyncRegistration
    {
        public uint StructSize;
        [MarshalAs(UnmanagedType.LPWStr)]
        public string ProviderName;
        [MarshalAs(UnmanagedType.LPWStr)]
        public string ProviderVersion;
        public IntPtr SyncRootIdentity;
        public uint SyncRootIdentityLength;
        public IntPtr FileIdentity;
        public uint FileIdentityLength;
        public Guid ProviderId;
    }

    private static class NativeMethods
    {
        [DllImport("cldapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        public static extern int CfRegisterSyncRoot(
            [MarshalAs(UnmanagedType.LPWStr)] string syncRootPath,
            ref CfSyncRegistration registration,
            ref CfSyncPolicies policies,
            int registerFlags);

        [DllImport("cldapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        public static extern int CfCreatePlaceholders(
            [MarshalAs(UnmanagedType.LPWStr)] string baseDirectoryPath,
            [In, Out] CfPlaceholderCreateInfo[] placeholderArray,
            uint placeholderCount,
            int createFlags,
            out uint entriesProcessed);

        [DllImport("cldapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        public static extern int CfConnectSyncRoot(
            [MarshalAs(UnmanagedType.LPWStr)] string syncRootPath,
            IntPtr callbackTable,
            IntPtr callbackContext,
            int connectFlags,
            out long connectionKey);

        [DllImport("cldapi.dll", ExactSpelling = true)]
        public static extern int CfDisconnectSyncRoot(long connectionKey);

        [DllImport("cldapi.dll", ExactSpelling = true)]
        public static extern int CfExecute(
            ref CfOperationInfo operationInfo,
            ref CfOperationParametersTransferData operationParameters);

        [DllImport("cldapi.dll", ExactSpelling = true)]
        public static extern int CfExecute(
            ref CfOperationInfo operationInfo,
            ref CfOperationParametersAckDelete operationParameters);

        [DllImport("cldapi.dll", ExactSpelling = true)]
        public static extern int CfExecute(
            ref CfOperationInfo operationInfo,
            ref CfOperationParametersAckRename operationParameters);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        public static extern void SHChangeNotify(
            uint wEventId,
            uint uFlags,
            [MarshalAs(UnmanagedType.LPWStr)] string? dwItem1,
            [MarshalAs(UnmanagedType.LPWStr)] string? dwItem2);
    }
}
