using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace IrisDrive.WindowsShell;

public sealed class DriveFolderPreparation
{
    public DriveFolderPreparation(string path, bool nativeSyncRootReady, string? warning)
    {
        Path = path;
        NativeSyncRootReady = nativeSyncRootReady;
        Warning = warning;
    }

    public string Path { get; }
    public bool NativeSyncRootReady { get; }
    public string? Warning { get; }
}

public static class WindowsCloudFiles
{
    private const string ProviderName = "Iris Drive";
    private const string ProviderVersion = "0.1";
    private const int CfRegisterFlagUpdate = 0x00000001;
    private const int CfRegisterFlagDisableOnDemandPopulationOnRoot = 0x00000002;
    private const int CfRegisterFlagMarkInSyncOnRoot = 0x00000004;
    private const ushort CfHydrationPolicyAlwaysFull = 3;
    private const ushort CfPopulationPolicyAlwaysFull = 3;
    private static readonly Guid ProviderId = new("2b58fb5d-b823-4d84-bd52-fcf9bd297fd4");

    public static string SyncRootPath =>
        System.IO.Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Iris Drive");

    public static DriveFolderPreparation EnsureSyncRoot()
    {
        var path = SyncRootPath;
        Directory.CreateDirectory(path);

        try
        {
            RegisterSyncRoot(path);
            return new DriveFolderPreparation(path, nativeSyncRootReady: true, warning: null);
        }
        catch (DllNotFoundException error)
        {
            return Fallback(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (EntryPointNotFoundException error)
        {
            return Fallback(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (Win32Exception error)
        {
            return Fallback(path, $"Cloud Files registration failed: {error.Message}");
        }
        catch (COMException error)
        {
            return Fallback(path, $"Cloud Files registration failed: {error.Message}");
        }
    }

    private static DriveFolderPreparation Fallback(string path, string warning) =>
        new(path, nativeSyncRootReady: false, warning);

    private static void RegisterSyncRoot(string path)
    {
        var identityBytes = Encoding.UTF8.GetBytes("iris-drive:main");
        var identity = Marshal.AllocHGlobal(identityBytes.Length);
        try
        {
            Marshal.Copy(identityBytes, 0, identity, identityBytes.Length);
            var registration = new CfSyncRegistration
            {
                StructSize = (uint)Marshal.SizeOf<CfSyncRegistration>(),
                ProviderName = ProviderName,
                ProviderVersion = ProviderVersion,
                SyncRootIdentity = identity,
                SyncRootIdentityLength = (uint)identityBytes.Length,
                FileIdentity = IntPtr.Zero,
                FileIdentityLength = 0,
                ProviderId = ProviderId,
            };
            var policies = new CfSyncPolicies
            {
                StructSize = (uint)Marshal.SizeOf<CfSyncPolicies>(),
                Hydration = new CfHydrationPolicy
                {
                    Primary = CfHydrationPolicyAlwaysFull,
                    Modifier = 0,
                },
                Population = new CfPopulationPolicy
                {
                    Primary = CfPopulationPolicyAlwaysFull,
                    Modifier = 0,
                },
                InSync = 0,
                HardLink = 0,
                PlaceholderManagement = 0,
            };
            var flags =
                CfRegisterFlagUpdate |
                CfRegisterFlagDisableOnDemandPopulationOnRoot |
                CfRegisterFlagMarkInSyncOnRoot;

            var hresult = NativeMethods.CfRegisterSyncRoot(path, ref registration, ref policies, flags);
            if (hresult >= 0)
            {
                return;
            }

            var createFlags = flags & ~CfRegisterFlagUpdate;
            var createHresult =
                NativeMethods.CfRegisterSyncRoot(path, ref registration, ref policies, createFlags);
            if (createHresult >= 0)
            {
                return;
            }

            throw new COMException(
                $"CfRegisterSyncRoot failed (update={FormatHResult(hresult)}, create={FormatHResult(createHresult)})",
                createHresult);
        }
        finally
        {
            Marshal.FreeHGlobal(identity);
        }
    }

    private static string FormatHResult(int hresult) => $"0x{hresult:X8}";

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
        public uint HardLink;
        public uint PlaceholderManagement;
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
    }
}
