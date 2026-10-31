using Test
using Store

@testset "store" begin
    @test Store.open_store("a") isa Store.Repo
end
