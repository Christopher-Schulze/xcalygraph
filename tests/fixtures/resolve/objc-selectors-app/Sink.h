#import <Foundation/Foundation.h>

@interface Sink : NSObject
- (void)send:(id)value to:(id)target;
- (void)send:(id)value on:(id)target;
- (void)run;
- (void)reset;
@end
