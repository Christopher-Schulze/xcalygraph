require_relative "store/repository"

class App
  def run
    repository = Store::MemoryRepository.new
    repository.put(Row.new(1))
    url = ENV.fetch("APP_URL")
    report(repository.find(1), url)
  end

  def report(row, url)
    puts "#{row} #{url}"
  end
end

Row = Struct.new(:id)
