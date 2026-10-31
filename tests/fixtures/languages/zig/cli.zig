const store = @import("store");
const Repo = store.Repo;
const Params = @import("repo.zig").Params;

pub fn rows(repo: *Repo) usize {
    return store.count(repo);
}

pub fn first(repo: *store.Repo, params: Params) ?[]const u8 {
    _ = params;
    return repo.get(1);
}
