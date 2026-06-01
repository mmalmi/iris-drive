using System;
using System.Threading;
using System.Windows;

namespace IrisDrive.WindowsShell;

public partial class App : System.Windows.Application
{
    private Mutex? appMutex;
    private bool ownsAppMutex;

    protected override void OnStartup(StartupEventArgs e)
    {
        appMutex = new Mutex(true, "IrisDrive.WindowsShell", out var created);
        if (!created)
        {
            appMutex.Dispose();
            appMutex = null;
            Shutdown();
            return;
        }
        ownsAppMutex = true;

        base.OnStartup(e);
        var window = new MainWindow();
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        if (ownsAppMutex)
        {
            appMutex?.ReleaseMutex();
        }
        appMutex?.Dispose();
        base.OnExit(e);
    }
}
