#import <Foundation/Foundation.h>

@class Row;

/// Reads rows by id.
@protocol Repository <NSObject>
- (Row *)find:(int)rowId;
@end

@interface Row : NSObject
@property(nonatomic) int rowId;
+ (Row *)rowWithId:(int)rowId;
@end

@interface MemoryRepository : NSObject <Repository>
- (Row *)find:(int)rowId;
- (void)put:(Row *)row;
- (void)put:(Row *)row replace:(BOOL)replace;
@end

@interface CachedRepository : MemoryRepository
@property(nonatomic, strong) MemoryRepository *backing;
- (void)flush;
@end
