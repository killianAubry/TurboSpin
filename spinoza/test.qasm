OPENQASM 2.0;
include "qelib1.inc";

qreg q[7];

h q[0];
h q[1];
h q[2];
h q[3];
h q[4];
h q[6];
cx q[0], q[1];
z q[3];
x q[4];
x q[0];
cx q[1], q[2];
cx q[4], q[5];
t q[0];
cx q[2], q[3];
h q[4];
h q[5];
h q[1];
t q[4];
cx q[3], q[4];
cx q[5], q[6];
s q[2];
h q[3];

// @columns [0,0,0,0,0,0,1,1,1,2,2,2,3,3,3,3,4,4,5,5,6,7]
