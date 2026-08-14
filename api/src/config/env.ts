import { z } from "zod";

const envSchema = z.object({
  DATABASE_URL: z
    .string()
    .default("postgres://crawler:crawler@localhost:5432/crawler"),
  REDIS_URL: z.string().default("redis://localhost:6379"),
  PORT: z.coerce.number().int().positive().default(3000),
  CRAWLER_URL: z.string().optional(),
});

export type AppConfig = z.infer<typeof envSchema>;

export function loadConfig(env: NodeJS.ProcessEnv = process.env): AppConfig {
  const parsed = envSchema.safeParse(env);
  if (!parsed.success) {
    throw new Error(
      `Invalid environment configuration: ${parsed.error.message}`,
    );
  }
  return parsed.data;
}
