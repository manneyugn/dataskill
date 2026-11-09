-- dataskill blocks this: agents never run writes.
DELETE FROM shop.orders WHERE status = 'pending';
