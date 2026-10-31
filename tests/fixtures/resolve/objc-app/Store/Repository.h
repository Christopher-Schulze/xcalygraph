#import <Foundation/Foundation.h>

@class Row;

/// Reads rows by id.
@protocol Repository <NSObject>
- (Row *)find:(int)rowId;
@end

@interface Row : NSObject
@property(nonatomic) int rowId;
@end

@interface MemoryRepository : NSObject <Repository>
- (Row *)find:(int)rowId;
- (void)put:(Row *)row;
@end
