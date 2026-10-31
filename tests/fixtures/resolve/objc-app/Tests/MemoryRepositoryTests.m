#import <XCTest/XCTest.h>
#import "../Store/Repository.h"

@interface MemoryRepositoryTests : XCTestCase
@end

@implementation MemoryRepositoryTests

- (void)testFindsStoredRow {
    MemoryRepository *repository = [[MemoryRepository alloc] init];
    Row *row = [[Row alloc] init];
    [repository put:row];
    XCTAssertNotNil([repository find:0]);
}

@end
