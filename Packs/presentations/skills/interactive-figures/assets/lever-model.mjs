/** Return ideal lever equilibrium for a 100 N load on a 1 m load arm.
 * @throws {RangeError} If the effort arm is not a finite number between 0.5 and 4 m.
 */
export function leverState(effortArm) {
  if (!Number.isFinite(effortArm) || effortArm < 0.5 || effortArm > 4) {
    throw new RangeError("Effort arm must be between 0.5 and 4 m.");
  }
  const loadForce = 100;
  const loadArm = 1;
  const effortForce = (loadForce * loadArm) / effortArm;
  return {
    effortArm,
    loadArm,
    loadForce,
    effortForce,
    moment: loadForce * loadArm,
    effortX: 300 - effortArm * 50,
    loadX: 300 + loadArm * 50,
    effortArrowTop: 199 - effortForce * 0.35,
    loadArrowTop: 199 - loadForce * 0.35,
  };
}
