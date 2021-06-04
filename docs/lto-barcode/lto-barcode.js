// for toolkit.js
function gettext(val) { return val; };

function draw_labels(target_id, label_list, page_layout, calibration) {
    let max_labels = compute_max_labels(page_layout);

    let count_fixed = 0;
    let count_fill = 0;

    for (i = 0; i < label_list.length; i++) {
	let item = label_list[i];
	if (item.end === null || item.end === '' || item.end === undefined) {
	    count_fill += 1;
	    continue;
	}
	if (item.end <= item.start) {
	    count_fixed += 1;
	    continue;
	}
	count_fixed += (item.end - item.start) + 1;
    }

    let rest = max_labels - count_fixed;
    let fill_size = 1;
    if (rest >= count_fill) {
	fill_size = Math.floor(rest/count_fill);
    }

    let list = [];

    let count_fill_2 = 0;

    for (i = 0; i < label_list.length; i++) {
	let item = label_list[i];
	let count;
	if (item.end === null || item.end === '' || item.end === undefined) {
	    count_fill_2 += 1;
	    if (count_fill_2 === count_fill) {
		count = rest;
	    } else {
		count = fill_size;
	    }
	    rest -= count;
	} else if (item.end <= item.start) {
	    count = 1;
	} else {
	    count = (item.end - item.start) + 1;
	}

	for (j = 0; j < count; j++) {
	    let id = item.start + j;

	    if (item.prefix.length == 6) {
		list.push({
		    label: item.prefix,
		    tape_type: item.tape_type,
		    mode: item.mode,
		    id: id,
		});
		rest += count - j - 1;
		break;
	    } else {
		let pad_len = 6-item.prefix.length;
		let label = item.prefix + id.toString().padStart(pad_len, 0);

		if (label.length != 6) {
		    rest += count - j;
		    break;
		}

		list.push({
		    label: label,
		    tape_type: item.tape_type,
		    mode: item.mode,
		    id: id,
		});
	    }
	}
    }

    generate_barcode_page(target_id, page_layout, list, calibration);
}

Ext.define('MainView', {
    extend: 'Ext.container.Viewport',
    alias: 'widget.mainview',

    layout: {
	type: 'vbox',
	align: 'stretch',
	pack: 'start',
    },
    width: 800,

    controller: {
	xclass: 'Ext.app.ViewController',

	update_barcode_preview: function() {
	    let me = this;
	    let view = me.getView();
	    let list_view = view.down("labelList");

	    let store = list_view.getStore();
	    let label_list = [];
	    store.each((record) => {
		label_list.push(record.data);
	    });

	    let page_layout_view = view.down("pageLayoutPanel");
	    let page_layout = page_layout_view.getValues();

	    let calibration_view = view.down("pageCalibration");
	    let page_calibration = calibration_view.getValues();

	    draw_labels("print_frame", label_list, page_layout, page_calibration);
	},

	update_calibration_preview: function() {
	    let me = this;
	    let view = me.getView();
	    let page_layout_view = view.down("pageLayoutPanel");
	    let page_layout = page_layout_view.getValues();

	    let calibration_view = view.down("pageCalibration");
	    let page_calibration = calibration_view.getValues();
	    console.log(page_calibration);
	    generate_calibration_page('print_frame', page_layout, page_calibration);
	},

	control: {
	    labelSetupPanel: {
		listchanged: function(store) {
		    this.update_barcode_preview();
		},
		activate: function() {
		    this.update_barcode_preview();
		},
	    },
	    pageLayoutPanel: {
		pagechanged: function(layout) {
		    this.update_barcode_preview();
		},
		activate: function() {
		    this.update_barcode_preview();
		},
	    },
	    pageCalibration: {
		calibrationchanged: function() {
		    this.update_calibration_preview();
		},
		activate: function() {
		    this.update_calibration_preview();
		},
	    },
	},
    },

    items: [
	{
	    xtype: 'tabpanel',
	    items: [
		{
		    xtype: 'labelSetupPanel',
		    title: 'Proxmox LTO Barcode Label Generator',
		    bodyPadding: 10,
		},
		{
		    xtype: 'pageLayoutPanel',
		    title: 'Page Layout',
		    bodyPadding: 10,
		},
		{
		    xtype: 'pageCalibration',
		    title: 'Printer Calibration',
		    bodyPadding: 10,
		},
	    ],
	},
	{
	    xtype: 'panel',
	    layout: "center",
	    title: 'Print Preview',
	    bodyStyle: "background-color: grey;",
	    bodyPadding: 10,
	    html: '<center><iframe id="print_frame" frameBorder="0"></iframe></center>',
	    border: false,
	    flex: 1,
	    scrollable: true,
	    tools: [{
		type: 'print',
		tooltip: 'Open Print Dialog',
		handler: function(event, toolEl, panelHeader) {
		    printBarcodePage();
		},
	    }],
	},
    ],
});

Ext.onReady(function() {
    Ext.create('MainView', {
	renderTo: Ext.getBody(),
    });
});
