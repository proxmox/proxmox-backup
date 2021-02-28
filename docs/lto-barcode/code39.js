// Code39 barcode generator
// see https://en.wikipedia.org/wiki/Code_39

// IBM LTO Ultrium Cartridge Label Specification
// http://www-01.ibm.com/support/docview.wss?uid=ssg1S7000429

const code39_codes = {
    "1": ['B', 's', 'b', 'S', 'b', 's', 'b', 's', 'B'],
    "A": ['B', 's', 'b', 's', 'b', 'S', 'b', 's', 'B'],
    "K": ['B', 's', 'b', 's', 'b', 's', 'b', 'S', 'B'],
    "U": ['B', 'S', 'b', 's', 'b', 's', 'b', 's', 'B'],

    "2": ['b', 's', 'B', 'S', 'b', 's', 'b', 's', 'B'],
    "B": ['b', 's', 'B', 's', 'b', 'S', 'b', 's', 'B'],
    "L": ['b', 's', 'B', 's', 'b', 's', 'b', 'S', 'B'],
    "V": ['b', 'S', 'B', 's', 'b', 's', 'b', 's', 'B'],

    "3": ['B', 's', 'B', 'S', 'b', 's', 'b', 's', 'b'],
    "C": ['B', 's', 'B', 's', 'b', 'S', 'b', 's', 'b'],
    "M": ['B', 's', 'B', 's', 'b', 's', 'b', 'S', 'b'],
    "W": ['B', 'S', 'B', 's', 'b', 's', 'b', 's', 'b'],

    "4": ['b', 's', 'b', 'S', 'B', 's', 'b', 's', 'B'],
    "D": ['b', 's', 'b', 's', 'B', 'S', 'b', 's', 'B'],
    "N": ['b', 's', 'b', 's', 'B', 's', 'b', 'S', 'B'],
    "X": ['b', 'S', 'b', 's', 'B', 's', 'b', 's', 'B'],

    "5": ['B', 's', 'b', 'S', 'B', 's', 'b', 's', 'b'],
    "E": ['B', 's', 'b', 's', 'B', 'S', 'b', 's', 'b'],
    "O": ['B', 's', 'b', 's', 'B', 's', 'b', 'S', 'b'],
    "Y": ['B', 'S', 'b', 's', 'B', 's', 'b', 's', 'b'],

    "6": ['b', 's', 'B', 'S', 'B', 's', 'b', 's', 'b'],
    "F": ['b', 's', 'B', 's', 'B', 'S', 'b', 's', 'b'],
    "P": ['b', 's', 'B', 's', 'B', 's', 'b', 'S', 'b'],
    "Z": ['b', 'S', 'B', 's', 'B', 's', 'b', 's', 'b'],

    "7": ['b', 's', 'b', 'S', 'b', 's', 'B', 's', 'B'],
    "G": ['b', 's', 'b', 's', 'b', 'S', 'B', 's', 'B'],
    "Q": ['b', 's', 'b', 's', 'b', 's', 'B', 'S', 'B'],
    "-": ['b', 'S', 'b', 's', 'b', 's', 'B', 's', 'B'],

    "8": ['B', 's', 'b', 'S', 'b', 's', 'B', 's', 'b'],
    "H": ['B', 's', 'b', 's', 'b', 'S', 'B', 's', 'b'],
    "R": ['B', 's', 'b', 's', 'b', 's', 'B', 'S', 'b'],
    ".": ['B', 'S', 'b', 's', 'b', 's', 'B', 's', 'b'],

    "9": ['b', 's', 'B', 'S', 'b', 's', 'B', 's', 'b'],
    "I": ['b', 's', 'B', 's', 'b', 'S', 'B', 's', 'b'],
    "S": ['b', 's', 'B', 's', 'b', 's', 'B', 'S', 'b'],
    " ": ['b', 'S', 'B', 's', 'b', 's', 'B', 's', 'b'],

    "0": ['b', 's', 'b', 'S', 'B', 's', 'B', 's', 'b'],
    "J": ['b', 's', 'b', 's', 'B', 'S', 'B', 's', 'b'],
    "T": ['b', 's', 'b', 's', 'B', 's', 'B', 'S', 'b'],
    "*": ['b', 'S', 'b', 's', 'B', 's', 'B', 's', 'b'],
};

const colors = [
    '#BB282E',
    '#FAE54A',
    '#9AC653',
    '#01A5E2',
    '#9EAAB6',
    '#D97E35',
    '#E27B99',
    '#67A945',
    '#F6B855',
    '#705A81',
];

const lto_label_width = 70;
const lto_label_height = 16.9;

function foreach_label(page_layout, callback) {
    let count = 0;
    let row = 0;
    let height = page_layout.margin_top;

    while ((height + page_layout.label_height) <= page_layout.page_height) {
	let column = 0;
	let width = page_layout.margin_left;

	while ((width + page_layout.label_width) <= page_layout.page_width) {
	    callback(column, row, count, width, height);
	    count += 1;

	    column += 1;
	    width += page_layout.label_width;
	    width += page_layout.column_spacing;
	}

	row += 1;
	height += page_layout.label_height;
	height += page_layout.row_spacing;
    }
}

function compute_max_labels(page_layout) {
    let max_labels = 0;
    foreach_label(page_layout, function() { max_labels += 1; });
    return max_labels;
}

function svg_label(mode, label, label_type, pagex, pagey, label_borders) {
    let svg = "";

    if (label.length !== 6) {
	throw "wrong label length";
    }
    if (label_type.length !== 2) {
	throw "wrong label_type length";
    }

    let ratio = 2.75;
    let parts = 3*ratio + 6; // 3*wide + 6*small;
    let barcode_width = (lto_label_width/12)*10; // 10*code + 2margin
    let small = barcode_width/(parts*10 + 9);
    let code_width = small*parts;
    let wide = small*ratio;
    let xpos = pagex + code_width;
    let height = 12;

    let label_rect = `x='${pagex}' y='${pagey}' width='${lto_label_width}' height='${lto_label_height}'`;

    if (mode === 'placeholder') {
	if (label_borders) {
	    svg += `<rect class='unprintable' ${label_rect} fill='none' style='stroke:black;stroke-width:0.1;'/>`;
	}
	return svg;
    }
    if (label_borders) {
	svg += `<rect ${label_rect} fill='none' style='stroke:black;stroke-width:0.1;'/>`;
    }

    if (mode === "color" || mode === "frame") {
	let w = lto_label_width/8;
	let h = lto_label_height - height;
	for (let i = 0; i < 7; i++) {
	    let textx = w/2 + pagex + i*w;
	    let texty = pagey;

	    let fill = "none";
	    if (mode === "color" && (i < 6)) {
		let letter = label.charAt(i);
		if (letter >= '0' && letter <= '9') {
		    fill = colors[parseInt(letter, 10)];
		}
	    }

	    svg += `<rect x='${textx}' y='${texty}' width='${w}' height='${h}' style='stroke:black;stroke-width:0.2;fill:${fill};'/>`;

	    if (i == 6) {
		textx += 3;
		texty += 3.7;
		svg += `<text x='${textx}' y='${texty}' style='font-weight:bold;font-size:3px;font-family:sans-serif;'>${label_type}</text>`;
	    } else {
		let letter = label.charAt(i);
		textx += 3.5;
		texty += 4;
		svg += `<text x='${textx}' y='${texty}' style='font-weight:bold;font-size:4px;font-family:sans-serif;'>${letter}</text>`;
	    }
	}
    }

    let raw_label = `*${label}${label_type}*`;

    for (let i = 0; i < raw_label.length; i++) {
	let letter = raw_label.charAt(i);

	let code = code39_codes[letter];
	if (code === undefined) {
	    throw `unable to encode letter '${letter}' with code39`;
	}

	if (mode === "simple") {
	    let textx = xpos + code_width/2;
	    let texty = pagey + 4;

	    if (i > 0 && (i+1) < raw_label.length) {
		svg += `<text x='${textx}' y='${texty}' style='font-weight:bold;font-size:4px;font-family:sans-serif;'>${letter}</text>`;
	    }
	}

	for (let c of code) {
	    if (c === 's') {
		xpos += small;
		continue;
	    }
	    if (c === 'S') {
		xpos += wide;
		continue;
	    }

	    let w = c === 'B' ? wide : small;
	    let ypos = pagey + lto_label_height - height;

	    svg += `<rect x='${xpos}' y='${ypos}' width='${w}' height='${height}' style='fill:black'/>`;
	    xpos = xpos + w;
	}
	xpos += small;
    }

    return svg;
}

function html_page_header() {
    let html = "<html5>";

    html += "<style>";

    /* no page margins */
    html += "@page{margin-left: 0px;margin-right: 0px;margin-top: 0px;margin-bottom: 0px;}";
    /* to hide things on printed page */
    html += "@media print { .unprintable { visibility: hidden;	} }";

    html += "</style>";

    //html += "<body onload='window.print()'>";
    html += "<body style='background-color: white;'>";

    return html;
}

function svg_page_header(page_width, page_height) {
    let svg = "<svg version='1.1' xmlns='http://www.w3.org/2000/svg'";
    svg += ` width='${page_width}mm' height='${page_height}mm' viewBox='0 0 ${page_width} ${page_height}'>`;

    return svg;
}

function printBarcodePage() {
    let frame = document.getElementById("print_frame");

    let window = frame.contentWindow;
    window.print();
}

function generate_barcode_page(target_id, page_layout, label_list, calibration) {
    let svg = svg_page_header(page_layout.page_width, page_layout.page_height);

    let c = calibration;

    console.log(calibration);

    svg += "<g id='barcode_page'";
    if (c !== undefined) {
	svg += ` transform='scale(${c.scalex}, ${c.scaley}),translate(${c.offsetx}, ${c.offsety})'`;
    }
    svg += '>';

    foreach_label(page_layout, function(column, row, count, xpos, ypos) {
	if (count >= label_list.length) { return; }

	let item = label_list[count];

	svg += svg_label(item.mode, item.label, item.tape_type, xpos, ypos, page_layout.label_borders);
    });

    svg += "</g>";
    svg += "</svg>";

    let html = html_page_header();
    html += svg;
    html += "</body>";
    html += "</html>";

    let frame = document.getElementById(target_id);

    setupPrintFrame(frame, page_layout.page_width, page_layout.page_height);

    let fwindow = frame.contentWindow;

    fwindow.document.open();
    fwindow.document.write(html);
    fwindow.document.close();
}

function setupPrintFrame(frame, page_width, page_height) {
    let dpi = 98;

    let dpr = window.devicePixelRatio;
    if (dpr !== undefined) {
	dpi = dpi*dpr;
    }

    let ppmm = dpi/25.4;

    frame.width = page_width*ppmm;
    frame.height = page_height*ppmm;
}

function generate_calibration_page(target_id, page_layout, calibration) {
    let frame = document.getElementById(target_id);

    setupPrintFrame(frame, page_layout.page_width, page_layout.page_height);

    let svg = svg_page_header(page_layout.page_width, page_layout.page_height);

    svg += "<defs>";
    svg += "<marker id='endarrow' markerWidth='10' markerHeight='7' ";
    svg += "refX='10' refY='3.5' orient='auto'><polygon points='0 0, 10 3.5, 0 7' />";
    svg += "</marker>";

    svg += "<marker id='startarrow' markerWidth='10' markerHeight='7' ";
    svg += "refX='0' refY='3.5' orient='auto'><polygon points='10 0, 10 7, 0 3.5' />";
    svg += "</marker>";
    svg += "</defs>";

    svg += "<rect x='50' y='50' width='100' height='100' style='fill:none;stroke-width:0.05;stroke:rgb(0,0,0)'/>";

    let text_style = "style='font-weight:bold;font-size:4;font-family:sans-serif;'";

    svg += `<text x='10' y='99' ${text_style}>Sx = 50mm</text>`;
    svg += "<line x1='0' y1='100' x2='50' y2='100' stroke='#000' marker-end='url(#endarrow)' stroke-width='.25'/>";

    svg += `<text x='60' y='99' ${text_style}>Dx = 100mm</text>`;
    svg += "<line x1='50' y1='100' x2='150' y2='100' stroke='#000' marker-start='url(#startarrow)' marker-end='url(#endarrow)' stroke-width='.25'/>";

    svg += `<text x='142' y='10' ${text_style} writing-mode='tb'>Sy = 50mm</text>`;
    svg += "<line x1='140' y1='0' x2='140' y2='50' stroke='#000' marker-end='url(#endarrow)' stroke-width='.25'/>";

    svg += `<text x='142' y='60' ${text_style} writing-mode='tb'>Dy = 100mm</text>`;
    svg += "<line x1='140' y1='50' x2='140' y2='150' stroke='#000' marker-start='url(#startarrow)' marker-end='url(#endarrow)' stroke-width='.25'/>";

    let c = calibration;
    if (c !== undefined) {
	svg += `<rect x='50' y='50' width='100' height='100' style='fill:none;stroke-width:0.05;stroke:rgb(255,0,0)' `;
	svg += `transform='scale(${c.scalex}, ${c.scaley}),translate(${c.offsetx}, ${c.offsety})'/>`;
    }

    svg += "</svg>";

    let html = html_page_header();
    html += svg;
    html += "</body>";
    html += "</html>";

    let fwindow = frame.contentWindow;

    fwindow.document.open();
    fwindow.document.write(html);
    fwindow.document.close();
}
