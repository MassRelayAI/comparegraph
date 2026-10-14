variable "region" {
  type    = string
  default = "eu-west-1"
}

resource "aws_s3_bucket" "logs" {
  bucket = "logs-${var.region}"
}

output "bucket_name" {
  value = aws_s3_bucket.logs.bucket
}
