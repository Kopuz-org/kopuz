-- Mute is session state like volume, kept apart from it so unmuting restores the level.
ALTER TABLE app_state ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;
