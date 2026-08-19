# Stage 1: build the Vite client
FROM node:20-alpine AS client
WORKDIR /build
COPY client/package.json client/package-lock.json* ./
RUN npm ci || npm install
COPY client/ ./
RUN npm run build

# Stage 2: runtime (Express serves client/dist + API)
FROM node:20-alpine
ENV NODE_ENV=production
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci --omit=dev || npm install --omit=dev
COPY server.js ./
COPY --from=client /build/dist ./client/dist
ENV PORT=3000
EXPOSE 3000
USER node
CMD ["node", "server.js"]