using System.IO;
using System.IO.Pipes;
using System.Reflection;
using System.Text;
using System.Text.Json;
using IrisDrive.WindowsShell;

var testRoot = Path.Combine(Path.GetTempPath(), "iris-drive-profile-test-" + Guid.NewGuid().ToString("N"));
var envNames = new[] { "IRIS_DRIVE_CONFIG_DIR", "IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR", "IRIS_DRIVE_WINDOWS_CLOUD_ROOT" };
var saved = envNames.ToDictionary(name => name, Environment.GetEnvironmentVariable);
try
{
    foreach (var name in envNames) Environment.SetEnvironmentVariable(name, null);
    Require(AppSetting("MutexName") == "IrisDrive.WindowsShell", "default mutex changed");
    Require(AppSetting("LaunchPipeName") == "IrisDrive.WindowsShell.LaunchArgs", "default pipe changed");
    var defaultRoot = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Iris Drive");
    Require(WindowsCloudFiles.SyncRootPath == defaultRoot, "default provider root changed");
    var defaultConfig = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "iris-drive");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", defaultConfig);
    Require(AppSetting("MutexName") == "IrisDrive.WindowsShell", "explicit default profile split its singleton");

    var profileA = Path.Combine(testRoot, "a");
    var profileB = Path.Combine(testRoot, "b");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileA);
    var mutexA = AppSetting("MutexName");
    var pipeA = AppSetting("LaunchPipeName");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", Path.Combine(profileA, ".") + Path.DirectorySeparatorChar);
    Require(AppSetting("MutexName") == mutexA && AppSetting("LaunchPipeName") == pipeA, "equal paths split a profile");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileA.ToUpperInvariant());
    Require(AppSetting("MutexName") == mutexA, "Windows path casing split a profile");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileB);
    var pipeB = AppSetting("LaunchPipeName");
    Require(AppSetting("MutexName") != mutexA && pipeB != pipeA, "different profiles share the singleton or launch pipe");
    Require(pipeA != "IrisDrive.WindowsShell.LaunchArgs" && pipeB != "IrisDrive.WindowsShell.LaunchArgs", "test must never open the user launch pipe");

    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileA);
    var startupWrites = 0;
    StartupService.SyncLaunchOnStartup(false, _ => startupWrites++);
    Require(startupWrites == 0, "alternate profile changed automatic startup registration");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", defaultConfig);
    StartupService.SyncLaunchOnStartup(false, _ => startupWrites++);
    Require(startupWrites == 1, "default profile stopped synchronizing startup preference");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileA);
    var metadata = typeof(WindowsCloudFiles).GetProperty("ConfigDirectoryPath", BindingFlags.NonPublic | BindingFlags.Static)!.GetValue(null);
    Require((string?)metadata == Path.GetFullPath(profileA), "provider metadata escaped the selected profile");
    var ownedRoot = Path.Combine(testRoot, "provider");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_WINDOWS_CLOUD_ROOT", ownedRoot);
    Require(WindowsCloudFiles.SyncRootPath == Path.GetFullPath(ownedRoot), "provider ignored the explicit owned root");
    var profileType = typeof(App).Assembly.GetType("IrisDrive.WindowsShell.WindowsProfileEnvironment")!;
    var identityA = profileType.GetProperty("SyncRootIdentity")!.GetValue(null);
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileB);
    Require(!Equals(identityA, profileType.GetProperty("SyncRootIdentity")!.GetValue(null)), "provider identities cross profiles");
    Environment.SetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR", profileA);
    foreach (var disabled in new[] { "0", "false", "OFF", "disabled", "none" })
    {
        Environment.SetEnvironmentVariable("IRIS_DRIVE_WINDOWS_CLOUD_ROOT", disabled);
        var preparation = WindowsCloudFiles.EnsureSyncRoot(Array.Empty<WindowsCloudFileEntry>(), _ => throw new Exception("disabled provider read a file"));
        Require(!preparation.NativeSyncRootReady && preparation.Path.Length == 0, "disabled provider created a root");
    }
    Require(!Directory.Exists(testRoot), "path-only checks unexpectedly created a provider directory");

    using var serverA = new NamedPipeServerStream(pipeA, PipeDirection.In, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
    using var serverB = new NamedPipeServerStream(pipeB, PipeDirection.In, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
    using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(5));
    var receiveA = Task.Run(async () =>
    {
        await serverA.WaitForConnectionAsync(deadline.Token);
        using var reader = new StreamReader(serverA, Encoding.UTF8);
        return JsonSerializer.Deserialize<string[]>(await reader.ReadToEndAsync(deadline.Token));
    });
    var receiveB = serverB.WaitForConnectionAsync(deadline.Token);
    var arguments = new[] { "https://drive.iris.to/approve-device/profile-a-only", "second argument" };
    typeof(App).GetMethod("SendLaunchArgumentsToPrimary", BindingFlags.NonPublic | BindingFlags.Static)!.Invoke(null, new object[] { arguments });
    var received = await receiveA;
    Require(received is not null && received.SequenceEqual(arguments), "secondary launch arguments changed");
    Require(!receiveB.IsCompleted, "secondary launch arguments reached another profile");
    deadline.Cancel();
    try { await receiveB; } catch (OperationCanceledException) { }
    Console.WriteLine("WINDOWS_PROFILE_ISOLATION_OK defaults paths provider-disable metadata identity and real launch-pipe routing");
}
finally
{
    foreach (var pair in saved) Environment.SetEnvironmentVariable(pair.Key, pair.Value);
}

static string AppSetting(string name)
{
    var flags = BindingFlags.NonPublic | BindingFlags.Static;
    return (string)(typeof(App).GetProperty(name, flags)?.GetValue(null) ?? typeof(App).GetField(name, flags)!.GetValue(null)!);
}

static void Require(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}

namespace IrisDrive.WindowsShell
{
    // The test links the production App/IPC code without constructing a GUI.
    public sealed class MainWindow : System.Windows.Window
    {
        public MainWindow(string[] arguments) { }
        public void ApplyLaunchArguments(string[] arguments) { }
    }
}
