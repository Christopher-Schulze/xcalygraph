module store_mod
  use util_mod
  implicit none
  type :: repo_t
    integer :: n = 0
  end type repo_t
contains
  subroutine open_store(r)
    type(repo_t), intent(inout) :: r
    r%n = helper(r%n)
    call reset(r)
  end subroutine open_store
  subroutine reset(r)
    type(repo_t), intent(inout) :: r
    r%n = 0
  end subroutine reset
end module store_mod
