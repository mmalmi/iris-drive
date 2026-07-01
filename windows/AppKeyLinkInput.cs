using System.Threading.Tasks;
using System.Windows;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private async void LinkSubmit_Click(object sender, RoutedEventArgs e)
    {
        await StartJoinRequestAsync();
    }

    private async Task StartJoinRequestAsync()
    {
        await RunSetupAsync(() => service.StartJoinRequestAsync());
    }
}
