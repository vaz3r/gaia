import type { NextFunction, Request, Response } from "express";

/** Consistent JSON error shape: { error, details? }. */
export function errorHandler(
  err: unknown,
  _req: Request,
  res: Response,
  _next: NextFunction,
): void {
  const message = err instanceof Error ? err.message : "internal server error";
  res.status(500).json({ error: message });
}
