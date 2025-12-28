#!/bin/bash
IMAGE_NAME="url-shortener"
IMAGE_TAG="latest"
INTERNAL_SERVER_PORT=3000
EXTERNAL_SERVER_PORT=3000

# Build Docker image
docker build -t ${IMAGE_NAME}:${IMAGE_TAG} .

# Stop and remove existing container if running
if docker ps -a | grep -q ${IMAGE_NAME}; then
  docker stop ${IMAGE_NAME}
  docker rm ${IMAGE_NAME}
fi

# Run container
# Note: Configure environment variables according to your deployment environment
docker run --name ${IMAGE_NAME} \
  --env-file .env \
  --cpus="1" \
  --memory="512m" \
  -d \
  -p ${EXTERNAL_SERVER_PORT}:${INTERNAL_SERVER_PORT} \
  --restart unless-stopped \
  ${IMAGE_NAME}:${IMAGE_TAG}

echo "Deployment complete. Container ${IMAGE_NAME} is running on port ${EXTERNAL_SERVER_PORT}"

