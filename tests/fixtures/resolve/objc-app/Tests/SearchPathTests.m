#import <XCTest/XCTest.h>
#import "Repository.h"

// Xcode header search paths: "Repository.h" lives in Store/, not next to this file.
@interface SearchPathTests : XCTestCase
@end

@implementation SearchPathTests

- (void)testPutThroughSearchPath {
    MemoryRepository *repository = [[MemoryRepository alloc] init];
    Row *row = [[Row alloc] init];
    [repository put:row];
    [repository put:row replace:YES];
    Row *made = [Row rowWithId:7];
}

@end
