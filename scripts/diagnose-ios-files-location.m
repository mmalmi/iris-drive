#import <FileProvider/FileProvider.h>
#import <Foundation/Foundation.h>
#import <objc/runtime.h>

// On the disposable simulator, record only the actual result of the app's
// public domain lookup. Leave the original completion and its results intact.
@interface IrisFilesLookupDiagnostics : NSObject
@end
@implementation IrisFilesLookupDiagnostics
+ (void)load {
    if (![[NSBundle mainBundle].bundleIdentifier isEqualToString:@"fi.siriusbusiness.drive"]) return;
    Class cls = NSFileProviderManager.class;
    SEL sel = @selector(getDomainsWithCompletionHandler:);
    Method method = class_getClassMethod(cls, sel);
    if (!method) return;
    typedef void (^Reply)(NSArray<NSFileProviderDomain *> *, NSError *);
    void (*original)(id, SEL, Reply) = (void *)method_getImplementation(method);
    IMP recorder = imp_implementationWithBlock(^(id target, Reply completion) {
        original(target, sel, ^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            const char *path = [[NSHomeDirectory() stringByAppendingPathComponent:@"Documents/iris-files-diagnostic.jsonl"] fileSystemRepresentation];
            FILE *file = fopen(path, "a");
            if (file) {
                NSError *cause = error.userInfo[NSUnderlyingErrorKey];
                NSDictionary *fields = @{@"domains": @(domains.count), @"enabled": @([domains filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"userEnabled == YES"]].count),
                  @"errorDomain": error.domain ?: @"", @"errorCode": @(error.code),
                  @"underlyingDomain": cause.domain ?: @"", @"underlyingCode": @(cause.code)};
                NSData *record = [NSJSONSerialization dataWithJSONObject:fields options:0 error:nil];
                if (record) { fwrite(record.bytes, 1, record.length, file); fputc('\n', file); }
                fclose(file);
            }
            completion(domains, error);
        });
    });
    method_setImplementation(method, recorder);
}
@end
