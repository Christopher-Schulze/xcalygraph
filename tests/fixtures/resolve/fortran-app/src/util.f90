module util_mod
  implicit none
contains
  integer function helper(n)
    integer, intent(in) :: n
    helper = n + 1
  end function helper
end module util_mod
