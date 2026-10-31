library(store)
test_that("opens", {
  expect_true(length(store::open_store("a")) > 0)
})
