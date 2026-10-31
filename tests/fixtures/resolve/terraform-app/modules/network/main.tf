variable "name" {}

resource "aws_vpc" "main" {
  tags = { Name = var.name }
}

output "vpc_id" {
  value = aws_vpc.main.id
}
