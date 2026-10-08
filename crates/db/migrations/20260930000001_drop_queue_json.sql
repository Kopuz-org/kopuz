-- The queue is rows now; the step between this and the last migration filled them from here.
ALTER TABLE queue_state DROP COLUMN queue_json;
ALTER TABLE queue_state DROP COLUMN shuffle_order_json;
