locals {
  azs = ["${var.aws_region}a", "${var.aws_region}b"]
}

# VPC
resource "aws_vpc" "this" {
  cidr_block           = var.vpc_cidr
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = merge(var.tags, { Name = "novapos-dev-vpc" })
}

resource "aws_internet_gateway" "igw" {
  vpc_id = aws_vpc.this.id
  tags   = merge(var.tags, { Name = "novapos-dev-igw" })
}

# Subnets
resource "aws_subnet" "public_a" {
  vpc_id                  = aws_vpc.this.id
  cidr_block              = cidrsubnet(var.vpc_cidr, 8, 1) # 10.0.1.0/24
  availability_zone       = local.azs[0]
  map_public_ip_on_launch = true
  tags                    = merge(var.tags, { Name = "novapos-dev-public-a" })
}

resource "aws_subnet" "private_a" {
  vpc_id            = aws_vpc.this.id
  cidr_block        = cidrsubnet(var.vpc_cidr, 8, 2) # 10.0.2.0/24
  availability_zone = local.azs[0]
  tags              = merge(var.tags, { Name = "novapos-dev-private-a" })
}

resource "aws_subnet" "private_b" {
  vpc_id            = aws_vpc.this.id
  cidr_block        = cidrsubnet(var.vpc_cidr, 8, 3) # 10.0.3.0/24
  availability_zone = local.azs[1]
  tags              = merge(var.tags, { Name = "novapos-dev-private-b" })
}

# Public route
resource "aws_route_table" "public" {
  vpc_id = aws_vpc.this.id
  tags   = merge(var.tags, { Name = "novapos-dev-rt-public" })
}

resource "aws_route" "public_igw" {
  route_table_id         = aws_route_table.public.id
  destination_cidr_block = "0.0.0.0/0"
  gateway_id             = aws_internet_gateway.igw.id
}

resource "aws_subnet_route_table_association" "public_a" {
  subnet_id      = aws_subnet.public_a.id
  route_table_id = aws_route_table.public.id
}

# Security Group for DB/Redis internal access
resource "aws_security_group" "internal" {
  name        = "novapos-dev-internal-sg"
  description = "Allow internal VPC access to DB/Redis"
  vpc_id      = aws_vpc.this.id

  ingress {
    description = "Postgres 5432"
    from_port   = 5432
    to_port     = 5432
    protocol    = "tcp"
    cidr_blocks = [var.vpc_cidr]
  }

  ingress {
    description = "Redis 6379"
    from_port   = 6379
    to_port     = 6379
    protocol    = "tcp"
    cidr_blocks = [var.vpc_cidr]
  }

  egress {
    description = "All egress"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = merge(var.tags, { Name = "novapos-dev-internal-sg" })
}

# RDS (PostgreSQL)
resource "aws_db_subnet_group" "db" {
  name       = "novapos-dev-db-subnet"
  subnet_ids = [aws_subnet.private_a.id, aws_subnet.private_b.id]
  tags       = merge(var.tags, { Name = "novapos-dev-db-subnet" })
}

resource "aws_db_instance" "postgres" {
  identifier             = "novapos-dev-postgres"
  engine                 = "postgres"
  engine_version         = "15.5"
  instance_class         = "db.t3.micro"
  allocated_storage      = 20
  db_name                = var.db_name
  username               = var.db_username
  password               = var.db_password
  db_subnet_group_name   = aws_db_subnet_group.db.name
  vpc_security_group_ids = [aws_security_group.internal.id]
  publicly_accessible    = false
  multi_az               = false
  skip_final_snapshot    = true
  apply_immediately      = true
  tags                   = merge(var.tags, { Name = "novapos-dev-postgres" })
}

# ElastiCache Redis (single node)
resource "aws_elasticache_subnet_group" "redis" {
  name       = "novapos-dev-redis-subnet"
  subnet_ids = [aws_subnet.private_a.id, aws_subnet.private_b.id]
}

resource "aws_elasticache_cluster" "redis" {
  cluster_id           = "novapos-dev-redis"
  engine               = "redis"
  node_type            = "cache.t3.micro"
  num_cache_nodes      = 1
  subnet_group_name    = aws_elasticache_subnet_group.redis.name
  security_group_ids   = [aws_security_group.internal.id]
  parameter_group_name = "default.redis7"
  tags                 = merge(var.tags, { Name = "novapos-dev-redis" })
}
