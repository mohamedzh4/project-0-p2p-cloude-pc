DCL hardware test - instructions
================================

This test starts a small Linux virtual computer on your PC. Someone else on
the same Wi-Fi then opens a link and uses that virtual computer in their
browser.

The FIRST run downloads Ubuntu (about 600 MB) and installs a desktop inside
the virtual computer: allow 15-45 minutes. Later runs start in a few minutes.

Nothing is installed on your PC except QEMU (the program that runs the
virtual computer). Its files live in one folder (see the end). Only devices
on your own Wi-Fi/LAN can reach it, and only with the password shown.

What you need
-------------
- A 64-bit PC with at least 4 CPU threads, 8 GB RAM, 20 GB free disk.
- Virtualization turned on in BIOS/UEFI (Intel VT-x or AMD-V/SVM).
- Internet on this PC.
- A second device (phone, laptop) on the SAME Wi-Fi.

Windows 10/11
-------------
1. Unzip this folder (right-click -> Extract All).
2. Double-click lan-test.cmd. Click "Yes" when Windows asks for permission.
3. If it says "Restart the PC", restart and double-click lan-test.cmd again.
4. Wait. When it is ready it shows a link, a username and a password.

Ubuntu Linux
------------
1. Unzip this folder.
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

If it says "did not answer"
---------------------------
Run it again: it reinstalls the desktop (takes as long as the first run).
Send the report file either way.

When you are done
-----------------
Press Ctrl+C in the test window (on Windows, then press Enter).
A file named lan-test-report-<date>.txt is saved in this folder.
Please send that file back. It contains no password.

Only checking the PC (no virtual computer):
  Windows:  lan-test.cmd -CheckOnly      (from a Command Prompt)
  Linux:    bash lan-test.sh --check

Removing everything afterwards
------------------------------
  Windows:  delete C:\dcl  (and QEMU in %LOCALAPPDATA%\dcl-host-agent)
  Linux:    rm -rf ~/.cache/dcl-lan-test ~/.cache/dcl-host-agent
