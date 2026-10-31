source("R/util.R")
library(dplyr)
open_store <- function(path) {
  url <- Sys.getenv("STORE_URL")
  rows <- helper(path)
  dplyr::filter(rows, id > 1)
}
test_that("opens", { expect_true(open_store("x")) })
