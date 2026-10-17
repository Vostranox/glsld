export interface Session {
  start(): Promise<void>;
  stop(): Promise<void>;
}

/** Serialize restarts so old processes and watchers finish before replacement. */
export class ServerLifecycle {
  private currentSession: Session | undefined;
  private pending: Promise<void> = Promise.resolve();
  private disposed = false;

  constructor(private readonly createSession: () => Promise<Session>) {}

  restart(): Promise<void> {
    return this.enqueue(async () => {
      if (this.disposed) {
        return;
      }

      await this.stopCurrentSession();
      if (this.disposed) {
        return;
      }

      const session = await this.createSession();
      this.currentSession = session;
      // Deactivation may have begun while the session was being created.
      if (this.disposed) {
        await this.stopCurrentSession();
        return;
      }

      try {
        await session.start();
      } catch (error) {
        await this.stopCurrentSession();
        throw error;
      }
    });
  }

  stop(): Promise<void> {
    this.disposed = true;
    return this.enqueue(() => this.stopCurrentSession());
  }

  private enqueue(operation: () => Promise<void>): Promise<void> {
    const next = this.pending.then(operation);
    // Report errors to the caller without blocking later operations.
    this.pending = next.catch(() => {});
    return next;
  }

  private async stopCurrentSession(): Promise<void> {
    const session = this.currentSession;
    this.currentSession = undefined;
    await session?.stop();
  }
}
