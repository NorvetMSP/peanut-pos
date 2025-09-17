FROM node:20 AS builder
WORKDIR /app
COPY frontends/admin-portal/package.json ./package.json
COPY frontends/admin-portal/package-lock.json ./package-lock.json
RUN npm ci
COPY frontends/admin-portal/ .
RUN npm run build

FROM nginx:alpine
RUN rm -rf /usr/share/nginx/html/*
COPY --from=builder /app/dist /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
