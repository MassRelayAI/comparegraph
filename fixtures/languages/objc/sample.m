#import <Foundation/Foundation.h>

static int bump(int n) {
    return n + 1;
}

@interface Counter : NSObject
- (int)add:(int)n;
@end

@implementation Counter
- (int)add:(int)n {
    return bump(n);
}
@end

int run(void) {
    Counter *c = [[Counter alloc] init];
    return [c add:2];
}
