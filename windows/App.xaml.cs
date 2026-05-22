using System;
using System.Threading;
using System.Windows;

namespace IrisDrive.WindowsShell;

public partial class App : System.Windows.Application
{
    private Mutex? appMutex;

    protected override void OnStartup(StartupEventArgs e)
    {
        appMutex = new Mutex(true, "IrisDrive.WindowsShell", out var created);
        if (!created)
        {
            Shutdown();
            return;
        }

        base.OnStartup(e);
        var window = new MainWindow();
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        appMutex?.ReleaseMutex();
        appMutex?.Dispose();
        base.OnExit(e);
    }
}
