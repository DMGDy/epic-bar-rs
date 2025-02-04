// nice
pub const CSS: &str = "
window {
    font-family: 'Fira Sans', sans-serif; 
} 

button {
    font-family: 'Fira Sans', sans-serif; 
    border-radius: 0px;
    margin: 0px;
    padding: 0px 3px; 
    color: white; 
    background-color: rgba(0,0,0,0);
}

box { 
    font-family: 'Fira Sans', sans-serif;
    border-radius: 0px;
    margin: 0px;
    padding: 0px 3px;
    color: white; 
}

top-bar {
    background-image: linear-gradient(
        to bottom,
        rgba(15,25,35,0.85)0%,
        rgba(20,30,40,0.85)40%,
        rgba(10,20,30,0.85)50%,
        rgba(5,15,25,0.85)70%
    );
}

status-reveal-button { 
    font-size: 22px; 
    border-right: 1px ridge white; 
    padding: 0px 4px 0px 0px;
} 

battery-icon { 
    padding: 0px 0px 0px 4px;
    font-size: 20px; 
    color: white;
} 

battery-label { 
    font-size:16px; 
    padding: 0px 0px; 
    color: white;
} 

mem-icon { 
    padding: 0px 0px 0px 4px;
    font-size: 20px; 
    color: white;
} 

mem-label { 
    font-size:16px; 
    padding: 0px 0px; 
    color: white;
} 


.occupied{
    background: linear-gradient(
        to bottom, 
        rgba(40,44,52,0.7), 
        rgba(30,33,40,0.7)
    );
    color: rgba(255,255,255,0.5);
    transition: all 0.2s ease; 
    text-shadow: 0 1px 1px rgba(0, 0, 0, 0.3);
}

.occupied:hover{
    background: linear-gradient(
        to bottom, 
        rgba(50,54,62,0.8),
        rgba(40,43,50,0.8));
}

.active {
    background: linear-gradient(
        to bottom, 
        rgba(65,105,225,0.8),
        rgba(45,85,205,0.8)
    ); 
    color: rgba(255,255,255,1);
    box-shadow: 
        inset 0 1px 0 rgba(255,255,255,0.1), 
        0 1px 3px rgba(0,0,0,0.2);
    transition: all 0.2s ease;
}

.active:hover{
    background: linear-gradient(
        to bottom, 
        rgba(75,115,235,0.9),
        rgba(55,95,215,0.9)
    );
}

.empty-active {
    background: linear-gradient(
        to bottom, 
        rgba(180,20,20,0.7),
        rgba(150,15,15,0.7)
    );
}

date-container { 
    border-left: 1px solid white; 
    font-size: 10px; padding: 0px 4px; color: white;
}

bottom-bar {
    background-image: linear-gradient(
        to bottom,
        rgba(15,25,35,0.85)0%,
        rgba(20,30,40,0.85)40%,
        rgba(10,20,30,0.85)50%,
        rgba(5,15,25,0.85)60%
    );
    padding:0px 0 4px 0;
}

tag-label {
    font-size: 12px; 
    color: white; 
    padding: 0px 3px; 
    border-right: 
    1px solid pink;
}

active-window-box {
    text-shadow: 1px 1px 4px white, 0 0 1em blue, 0 0 0.2em blue; \
    transition-duration: .3s;
    color: white; 
    font-size: 14px;
}

active-window-box:hover {
    box-shadow: 0 0 8px white, 0 0 10px white, 0 0 6px red, 0 0 10px blue;
}

window-box {
    transition-duration: .3s; 
    color: white; 
    font-size: 14px; 
    border-right: 1px solid white;
}

window-box:hover {
    box-shadow: 0 0 8px white, 0 0 10px white, 0 0 6px red, 0 0 10px blue; 
}

window-box-empty {
    color: purple; 
    text-shadow: 1px 1px 3px red; 
    font-size: 18px; 
    padding: 0px 4px 0px 0px;
}

icon-image {
    padding: 0px 4px 0px 0px;
}

workspace-window-box-empty {
    margin: 0px 8px 0px 2px;
    border: 1px solid red;
    box-shadow: 0 0 4px white, 0 0 6px white, 0 0 10px red, 0 0 2px blue;
}

workspace-window-box-active {
    margin: 0px 8px 0px 2px;
    border: 1px solid cyan;
    box-shadow: 0 0 4px white, 0 0 6px white, 0 0 6px red, 0 0 10px blue;
}

workspace-window-box { 
    border: 1px solid white; 
    margin: 0px 4px;
} 


";


