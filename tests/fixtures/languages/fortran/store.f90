module store_mod
  use util_mod, only: helper
  implicit none
  type :: repo_t
    integer :: n
  end type repo_t
contains
  subroutine open_store(r)
    type(repo_t), intent(inout) :: r
    character(len=64) :: url
    call helper(r%n)
    call get_environment_variable("STORE_URL", url)
    r%n = count_rows(r)
  end subroutine open_store
  integer function count_rows(r)
    type(repo_t) :: r
    count_rows = r%n
  end function count_rows
end module store_mod
program main
  use store_mod
  include "consts.inc"
  call open_store(x)
end program main
