import QtQuick

// Small, original line icons; independent of installed fonts and icon themes.
Canvas {
    id: icon
    property string symbol: "file"
    property color ink: "#6d4aff"
    implicitWidth: 22; implicitHeight: 22
    onSymbolChanged: requestPaint()
    onInkChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onPaint: {
        const c = getContext("2d")
        c.reset(); c.scale(width/24,height/24)
        c.strokeStyle=ink; c.fillStyle=ink; c.lineWidth=1.7; c.lineCap="round"; c.lineJoin="round"
        function line(points) { c.beginPath(); c.moveTo(points[0],points[1]); for(let i=2;i<points.length;i+=2)c.lineTo(points[i],points[i+1]); c.stroke() }
        function circle(x,y,r) { c.beginPath(); c.arc(x,y,r,0,Math.PI*2); c.stroke() }
        if(symbol==="folderTile") {
            c.globalAlpha=0.65; c.beginPath(); c.moveTo(2,5); c.lineTo(9,5); c.lineTo(12,8); c.lineTo(22,8); c.lineTo(22,20); c.lineTo(2,20); c.closePath(); c.fill();
            c.globalAlpha=1; c.beginPath(); c.rect(2,9,20,12); c.fill();
        }
        else if(symbol==="folder") { line([3,7,3,19,21,19,21,7,12,7,10,4,3,4,3,7]); }
        else if(symbol==="photo") { c.strokeRect(3,4,18,16); circle(8,9,2); line([3,18,10,12,14,16,18,11,21,15]); }
        else if(symbol==="computer") { c.strokeRect(3,4,18,12); line([8,21,16,21]); line([12,16,12,21]); }
        else if(symbol==="trash") { line([4,6,20,6]); line([6,6,7,21,17,21,18,6]); line([9,6,9,3,15,3,15,6]); line([10,10,10,17]); line([14,10,14,17]); }
        else if(symbol==="sync") { line([4,8,18,8,15,5]); line([20,16,6,16,9,19]); }
        else if(symbol==="refresh") { c.beginPath(); c.arc(12,12,8,.4,5.5); c.stroke(); line([18,3,18,8,13,8]); }
        else if(symbol==="cloud") { c.beginPath(); c.moveTo(7,19); c.bezierCurveTo(-1,19,0,9,7,10); c.bezierCurveTo(6,0,20,1,19,11); c.bezierCurveTo(25,12,24,19,18,19); c.closePath(); c.stroke(); }
        else if(symbol==="shared") { circle(9,7,3); c.beginPath(); c.arc(9,19,6,Math.PI,0); c.stroke(); line([17,6,20,9,17,12]); line([16,16,20,16]); }
        else if(symbol==="activity") { line([2,12,6,12,9,5,14,20,18,12,22,12]); }
        else if(symbol==="settings") { for(let y=6;y<22;y+=6)line([3,y,21,y]); circle(8,6,2); circle(16,12,2); circle(10,18,2); }
        else if(symbol==="search") { circle(10,10,6); line([15,15,21,21]); }
        else if(symbol==="brand") { line([3,18,3,6,8,6,16,18,21,18,21,6]); }
        else { line([6,3,14,3,19,8,19,21,6,21,6,3]); line([14,3,14,8,19,8]); line([9,13,16,13]); line([9,17,14,17]); }
    }
}
