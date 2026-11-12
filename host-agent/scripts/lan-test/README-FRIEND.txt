DCL hardware test - instructions
================================

This test starts a small Linux virtual computer on your PC. Someone else on
the same Wi-Fi then opens a link and uses that virtual computer in their
browser. It takes about 10-30 minutes the first time.

Nothing is installed permanently except QEMU (the program that runs the
virtual computer). The virtual computer is deleted when you stop the test.
Only devices on your own Wi-Fi/LAN can reach it, and only with the password.

What you need
-------------
- A 64-bit PC with at least 4 CPU threads, 8 GB RAM, 80 GB free disk.
- Virtualization turned on in BIOS/UEFI (Intel VT-x or AMD-V/SVM).
- rootfs.qcow2 placed in this folder (unless you were told it downloads).
- A second device (phone, laptop) on the SAME Wi-Fi.

Windows 10/11
-------------
1. Unzip this folder (right-click -> Extract All). Put rootfs.qcow2 in it.
2. Double-click lan-test.cmd. Click "Yes" when Windows asks for permission.
3. If it says "Restart the PC", restart and double-click lan-test.cmd again.
4. Wait. When it is ready it shows a link, a username and a password.

Ubuntu Linux
------------
1. Unzip this folder. Put rootfs.qcow2 in it.
2. Open a terminal in this folder and run:   bash lan-test.sh
3. Type your password if sudo asks.
4. Wait. When it is ready it shows a link, a username and a password.

On the second device
--------------------
1. Open the link (https://192.168.x.x:8443) in Chrome, Edge or Firefox.
2. The browser warns "not private / not secure". This is expected for this
   test. Click "Advanced" then "Continue" / "Accept the risk".
3. Log in with username "ubuntu" and the password shown on the first PC.
4. You should see a desktop. Open the terminal or a few windows, try it.

When you are done
-----------------
Press Ctrl+C in the test window (on Windows, then press Enter).
A file named lan-test-report-<date>.txt is saved in this folder.
Please send that file back. It contains no password.

Only checking the PC (no virtual computer):
  Windows:  lan-test.cmd -CheckOnly
  Linux:    bash lan-test.sh --check
