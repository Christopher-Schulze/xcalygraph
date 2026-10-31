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

- (void)put:(Row *)row replace:(BOOL)replace {
    rows[@(row.rowId)] = row;
}

@end

@implementation Row (Factory)

+ (Row *)rowWithId:(int)rowId {
    Row *row = [[Row alloc] init];
    row.rowId = rowId;
    return row;
}

@end

@implementation CachedRepository

- (Row *)find:(int)rowId {
    Row *row = [super find:rowId];
    [self.backing put:row];
    return row;
}

- (void)flush {
}

@end
