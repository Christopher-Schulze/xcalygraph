module Store
  # Reads rows by id.
  module Readable
    def find(id)
      raise NotImplementedError
    end
  end

  class Base
    def check(id)
      raise ArgumentError unless valid?(id)
    end

    def valid?(id)
      id.positive?
    end
  end

  class MemoryRepository < Base
    include Readable

    def initialize
      @rows = {}
    end

    def find(id)
      check(id)
      @rows[id]
    end

    def put(row)
      @rows[row.id] = row
    end
  end
end
