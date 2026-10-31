open_store <- function(path) {
  rows <- read_rows(path)
  jsonlite::toJSON(rows)
}

read_rows <- function(path) {
  url <- Sys.getenv("STORE_URL")
  lapply(c(path, url), normalize)
}
