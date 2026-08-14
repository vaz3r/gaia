# Build + run the gaia admin/search API.
# Multi-stage: install deps + compile TS, then a slim runtime.
# --ignore-scripts: the production build only needs `tsc` + `node`; esbuild's
# postinstall native binary is not required at runtime.
FROM node:22-alpine AS builder
WORKDIR /app
COPY package.json pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile --ignore-scripts
COPY tsconfig.json ./
COPY src ./src
RUN pnpm build

FROM node:22-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY package.json pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile --prod --ignore-scripts
COPY --from=builder /app/dist ./dist
USER node
EXPOSE 3000
CMD ["node", "dist/index.js"]
