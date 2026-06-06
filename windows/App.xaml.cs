using System;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using System.Threading;
using System.Windows;

namespace IrisDrive.WindowsShell;

public partial class App : System.Windows.Application
{
    private const string MutexName = "IrisDrive.WindowsShell";
    private const string LaunchPipeName = "IrisDrive.WindowsShell.LaunchArgs";
    private Mutex? appMutex;
    private bool ownsAppMutex;
    private CancellationTokenSource? launchPipeCancellation;

    protected override void OnStartup(StartupEventArgs e)
    {
        appMutex = new Mutex(true, MutexName, out var created);
        if (!created)
        {
            SendLaunchArgumentsToPrimary(e.Args);
            appMutex.Dispose();
            appMutex = null;
            Shutdown();
            return;
        }
        ownsAppMutex = true;

        base.OnStartup(e);
        var window = new MainWindow(e.Args);
        StartLaunchArgumentPipe(window);
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        launchPipeCancellation?.Cancel();
        launchPipeCancellation?.Dispose();
        if (ownsAppMutex)
        {
            appMutex?.ReleaseMutex();
        }
        appMutex?.Dispose();
        base.OnExit(e);
    }

    private static void SendLaunchArgumentsToPrimary(string[] arguments)
    {
        if (arguments.Length == 0)
        {
            return;
        }

        try
        {
            using var client = new NamedPipeClientStream(
                ".",
                LaunchPipeName,
                PipeDirection.Out,
                PipeOptions.Asynchronous);
            client.Connect(750);
            using var writer = new StreamWriter(client, new UTF8Encoding(false));
            writer.Write(JsonSerializer.Serialize(arguments));
        }
        catch
        {
            // The existing instance may still be starting; do not block process startup here.
        }
    }

    private void StartLaunchArgumentPipe(MainWindow window)
    {
        launchPipeCancellation = new CancellationTokenSource();
        var token = launchPipeCancellation.Token;
        _ = Task.Run(async () =>
        {
            while (!token.IsCancellationRequested)
            {
                try
                {
                    using var server = new NamedPipeServerStream(
                        LaunchPipeName,
                        PipeDirection.In,
                        1,
                        PipeTransmissionMode.Byte,
                        PipeOptions.Asynchronous);
                    await server.WaitForConnectionAsync(token);
                    using var reader = new StreamReader(server, Encoding.UTF8);
                    var payload = await reader.ReadToEndAsync(token);
                    var arguments = JsonSerializer.Deserialize<string[]>(payload) ?? Array.Empty<string>();
                    window.Dispatcher.Invoke(() => window.ApplyLaunchArguments(arguments));
                }
                catch (OperationCanceledException)
                {
                    break;
                }
                catch
                {
                    // Keep the primary app alive even if a malformed launch payload arrives.
                }
            }
        }, token);
    }
}
