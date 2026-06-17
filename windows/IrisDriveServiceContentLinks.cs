using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed partial class IrisDriveService
{
    public Task<IrisDriveStatusData> ImportContentLinkAsync(string link)
    {
        if (string.IsNullOrWhiteSpace(link))
        {
            throw new InvalidOperationException("Content link is required.");
        }

        return DispatchNativeActionAsync(
            new Dictionary<string, object>
            {
                ["type"] = "import_content_link",
                ["link"] = link.Trim(),
            });
    }
}
