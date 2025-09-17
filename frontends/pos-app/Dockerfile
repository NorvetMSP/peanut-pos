FROM node:20 AS builder
WORKDIR /app
COPY frontends/pos-app/package.json ./package.json
COPY frontends/pos-app/package-lock.json ./package-lock.json
RUN npm ci
COPY frontends/pos-app/ .
RUN npm run build    # outputs to dist/ (Vite) or build/ (CRA) - adjust COPY below if CRA

FROM nginx:alpine
RUN rm -rf /usr/share/nginx/html/*
COPY --from=builder /app/dist /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
