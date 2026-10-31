#import "Repository.h"

@implementation Row
@end

@implementation MemoryRepository {
    NSMutableDictionary *rows;
}

- (BOOL)valid:(int)rowId {
    return rowId >= 0;
}

- (Row *)find:(int)rowId {
    if (![self valid:rowId]) {
        return nil;
    }
    return rows[@(rowId)];
}

- (void)put:(Row *)row {
    rows[@(row.rowId)] = row;
}

@end
