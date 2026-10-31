variable "region" {
  default = "eu-west-1"
}

locals {
  name = "app-${var.region}"
}

module "network" {
  source = "./modules/network"
  name   = local.name
}

resource "aws_s3_bucket" "logs" {
  bucket = local.name
  vpc    = module.network.vpc_id
}
