#import "Sink.h"
@implementation Sink
- (void)send:(id)value to:(id)target {}
- (void)send:(id)value on:(id)target {}
- (void)run {
    [self send:@"x" to:self];
    [self send:@"x" on:self];
    [self send:@"x" through:self];
    SEL selected = @selector(send:on:);
    SEL missing = @selector(send:through:);
    [self reset];
    SEL resetSelector = @selector(reset);
}
- (void)reset {}
@end
