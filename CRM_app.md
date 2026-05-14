Create a cross-platform desktop application using Tauri 2 — a modern, distraction-free sqlite3 database interraction app. 


Core Features
the GUI should be made using Fast Light Toolkit and the backend will use  sqlite3. 
Both will run in their own thread, w/o interraction other than posting tasks to each other. 

Both frontend as well as the backend will have a SPSC atomic queue, from which they get their tasks - attention at false sharing.  
Both will have implemented a backoff/retry policy. 

THe app will also have a logging policy (console, file and no logging). 

Policies are defined as static behaviour, so that the app can be shaped as would be in C++ like that App template<typename Logging Policy, Retry Policy,...>. 


View: 
The main screen will be a panel starting in the very first middle of the screen. It will have 4 tabs, firtst tab named "Companie", then "Contacts", then "Activities" and a fourth "Logs", which will store daily logs inputed by the user. 


the companies tab will display all the companies record in the notes_app.db stored in the data folder. 
Upon clicking on a record, it will open a company window displaying the company details, that can also edit details. Simultaneously, it will open the explorer to the path: "Company name/year/month" like e.g Dea/2026/05. 

comment heavily the code. 

Instruct how to isntall rust and tauri framework, i do not have python and i dont want to. Use npm only. 
