const std = @import("std");

pub const Repo = struct {
    rows: std.AutoHashMap(u32, []const u8),

    pub fn get(self: *Repo, id: u32) ?[]const u8 {
        return check(self.rows.get(id));
    }
};

fn check(row: ?[]const u8) ?[]const u8 {
    return row;
}

pub fn count(repo: *Repo) usize {
    return repo.rows.count();
}

test "check passes rows" {
    try std.testing.expect(check(null) == null);
}

pub const Params = struct {
    len: usize,
};
