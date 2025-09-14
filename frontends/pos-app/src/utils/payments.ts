// src/utils/payments.ts
export async function simulatePayment(method: 'card' | 'cash' | 'crypto', amount: number): Promise<void> {
  console.log(`Simulating ${method} payment for $${amount.toFixed(2)}`);
  // Simulate different processing time based on method (just for demonstration)
  let delay = 1000;
  if (method === 'cash') {
    delay = 500;   // cash is quick (no authorization needed)
  } else if (method === 'crypto') {
    delay = 1500;  // crypto might be a bit slower
  }
  return new Promise((resolve) => setTimeout(resolve, delay));
}
