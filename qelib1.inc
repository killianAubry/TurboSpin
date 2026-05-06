OPENQASM 2.0;
"""
Standard OpenQASM gate definitions
"""

gate id q { U(0, 0, 0) q; }
gate x q { U(pi, 0, pi) q; }
gate y q { U(pi, pi/2, pi/2) q; }
gate z q { U(0, 0, pi) q; }
gate h q { U(pi/2, 0, pi) q; }
gate s q { U(0, 0, pi/2) q; }
gate t q { U(0, 0, pi/4) q; }
gate sdg q { U(0, 0, -pi/2) q; }
gate tdg q { U(0, 0, -pi/4) q; }

gate rx(theta) q { U(theta, -pi/2, pi/2) q; }
gate ry(theta) q { U(theta, 0, 0) q; }
gate rz(phi) q { U(0, 0, phi) q; }

gate cx q1, q2 { CX q1, q2; }
gate CX q1, q2 { CTRL @ U(pi, 0, pi) q1, q2; }
gate cz q1, q2 { CTRL @ Z q1, q2; }
gate ccx q1, q2, q3 { CCX q1, q2, q3; }
gate swap q1, q2 { CX q1, q2; CX q2, q1; CX q1, q2; }

gate u1(lambda) q { U(0, 0, lambda) q; }
gate u2(phi, lambda) q { U(pi/2, phi, lambda) q; }
gate u3(theta, phi, lambda) q { U(theta, phi, lambda) q; }

gate p(lambda) q { U(0, 0, lambda) q; }
