module Store
  # Calls without parentheses or receiver look like variables.
  class Limits
    def check(count)
      total = count + reserve
      if total > maximum
        reset
      end
      return total if ready
      total
    end

    def reserve
      1
    end

    def maximum
      10
    end

    def reset
      nil
    end

    def ready
      true
    end
  end
end
