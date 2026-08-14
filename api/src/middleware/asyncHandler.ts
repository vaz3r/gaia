import type { NextFunction, Request, RequestHandler, Response } from "express";

type AsyncHandler = (
  req: Request,
  res: Response,
  next: NextFunction,
) => Promise<unknown>;

/** Wrap an async route handler so rejections reach the error middleware
 * instead of crashing the process (Express 4 does not catch async errors). */
export function asyncHandler(fn: AsyncHandler): RequestHandler {
  return (req, res, next) => {
    fn(req, res, next).catch(next);
  };
}
