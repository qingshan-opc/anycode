-- Every plan now includes 10 team seats by default (small-team friendly).
-- Extra seats are sold as yearly add-ons (entitlements.extra_seats).
UPDATE cloud_plans SET seat_limit = 10 WHERE id IN ('free', 'cloud_5h', 'pro') AND seat_limit < 10;

UPDATE entitlements SET seat_limit = 10 WHERE seat_limit < 10;

ALTER TABLE entitlements ADD COLUMN extra_seats INT NOT NULL DEFAULT 0 AFTER seat_limit;
ALTER TABLE entitlements ADD COLUMN extra_seats_until DATE NULL AFTER extra_seats;

ALTER TABLE payment_orders ADD COLUMN quantity INT NULL AFTER billing_cycle;
