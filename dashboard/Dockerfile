# syntax=docker/dockerfile:1

# Build the Vite dashboard, then serve the static bundle via nginx with an
# API proxy so the dashboard talks to the gaia API on the same origin.
FROM node:22-alpine AS builder
WORKDIR /app
COPY package.json pnpm-lock.yaml ./
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    corepack enable && pnpm install --frozen-lockfile --ignore-scripts
COPY tsconfig.json vite.config.ts index.html postcss.config.js tailwind.config.ts ./
COPY src ./src
RUN pnpm build

FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/dist /usr/share/nginx/html
EXPOSE 80
