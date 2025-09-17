variable "aws_region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}

variable "vpc_cidr" {
  description = "VPC CIDR"
  type        = string
  default     = "10.0.0.0/16"
}

variable "db_name" {
  description = "Default Postgres DB name"
  type        = string
  default     = "novapos"
}

variable "db_username" {
  description = "Postgres username"
  type        = string
  default     = "novapos"
}

variable "db_password" {
  description = "Postgres password (dev only)"
  type        = string
  default     = "novapos"
  sensitive   = true
}

variable "tags" {
  type        = map(string)
  default     = { Project = "NovaPOS", Env = "dev" }
  description = "Common resource tags"
}
