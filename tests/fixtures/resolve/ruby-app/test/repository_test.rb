require "minitest/autorun"
require_relative "../lib/store/repository"

class RepositoryTest < Minitest::Test
  def test_find
    repository = Store::MemoryRepository.new
    repository.put(1)
    assert repository.find(1)
  end
end
