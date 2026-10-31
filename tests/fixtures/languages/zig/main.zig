const std = @import("std");
const repo = @import("repo.zig");

pub fn main() !void {
    var store = repo.Repo{ .rows = std.AutoHashMap(u32, []const u8).init(std.heap.page_allocator) };
    _ = store.get(1);
    std.debug.print("{d}\n", .{repo.count(&store)});
}
