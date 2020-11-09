Ext.define('PBS.TaskSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsTaskSummary',

    title: gettext('Task Summary'),

    states: [
	"",
	"error",
	"warning",
	"ok",
    ],

    types: [
	"backup",
	"prune",
	"garbage_collection",
	"sync",
	"verify",
    ],

    titles: {
	"backup": gettext('Backups'),
	"prune": gettext('Prunes'),
	"garbage_collection": gettext('Garbage collections'),
	"sync": gettext('Syncs'),
	"verify": gettext('Verify'),
    },

    controller: {
	xclass: 'Ext.app.ViewController',


	openTaskList: function(grid, td, cellindex, record, tr, rowindex) {
	    let me = this;
	    let view = me.getView();

	    if (cellindex > 0) {
		let tasklist = view.tasklist;
		let state = view.states[cellindex];
		let type = view.types[rowindex];
		let filterParam = {
		    limit: 0,
		    'statusfilter': state,
		    'typefilter': type,
		};

		if (me.since) {
		    filterParam.since = me.since;
		}

		if (record.data[state] === 0) {
		    return;
		}

		if (tasklist === undefined) {
		    tasklist = Ext.create('Ext.grid.Panel', {
			tools: [{
			    handler: () => tasklist.setVisible(false),
			}],
			floating: true,
			scrollable: true,

			height: 400,
			width: 600,

			columns: [
			    {
				text: gettext('Task'),
				dataIndex: 'upid',
				renderer: Proxmox.Utils.render_upid,
				flex: 1,
			    },
			    {
				header: gettext("Start Time"),
				dataIndex: 'starttime',
				width: 130,
				renderer: function(value) {
				    return Ext.Date.format(value, "M d H:i:s");
				},
			    },
			    {
				xtype: 'actioncolumn',
				width: 40,
				items: [
				    {
					iconCls: 'fa fa-chevron-right',
					tooltip: gettext('Open Task'),
					handler: function(g, rowIndex) {
					    let rec = tasklist.getStore().getAt(rowIndex);
					    tasklist.setVisible(false);
					    Ext.create('Proxmox.window.TaskViewer', {
						upid: rec.data.upid,
						endtime: rec.data.endtime,
						listeners: {
						    close: () => tasklist.setVisible(true),
						},
					    }).show();
					},
				    },
				],
			    },
			],

			store: {
			    sorters: [
				{
				    property: 'starttime',
				    direction: 'DESC',
				},
			    ],
			    type: 'store',
			    model: 'proxmox-tasks',
			    proxy: {
				type: 'proxmox',
				url: "/api2/json/nodes/localhost/tasks",
			    },
			},
		    });

		    view.on('destroy', function() {
			tasklist.setVisible(false);
			tasklist.destroy();
			tasklist = undefined;
		    });

		    view.tasklist = tasklist;
		} else {
		    let cidx = tasklist.cidx;
		    let ridx = tasklist.ridx;

		    if (cidx === cellindex && ridx === rowindex && tasklist.isVisible()) {
			tasklist.setVisible(false);
			return;
		    }
		}

		tasklist.cidx = cellindex;
		tasklist.ridx = rowindex;

		let task = view.titles[type];
		let status = "";
		switch (state) {
		    case 'ok': status = gettext("OK"); break;
		    case 'warnings': status = gettext("Warning"); break;
		    case 'error': status = Proxmox.Utils.errorText; break;
		}
		let icon = me.render_icon(state, 1);
		tasklist.setTitle(`${task} - ${status} ${icon}`);
		tasklist.getStore().getProxy().setExtraParams(filterParam);
		tasklist.getStore().removeAll();

		tasklist.showBy(td, 'bl-tl');
		setTimeout(() => tasklist.getStore().reload(), 10);
	    }
	},

	render_icon: function(state, count) {
	    let cls = 'question';
	    let color = 'faded';
	    switch (state) {
		case "error":
		    cls = "times-circle";
		    color = "critical";
		    break;
		case "warning":
		    cls = "exclamation-circle";
		    color = "warning";
		    break;
		case "ok":
		    cls = "check-circle";
		    color = "good";
		    break;
		default: break;
	    }

	    if (count < 1) {
		color = "faded";
	    }
	    cls += " " + color;
	    return `<i class="fa fa-${cls}"></i>`;
	},

	render_count: function(value, md, record, rowindex, colindex) {
	    let me = this;
	    let view = me.getView();
	    let icon = me.render_icon(view.states[colindex], value);
	    return `${icon} ${value}`;
	},
    },

    updateTasks: function(source, since) {
	let me = this;
	let controller = me.getController();
	let data = [];
	me.types.forEach((type) => {
	    source[type].type = me.titles[type];
	    data.push(source[type]);
	});
	me.lookup('grid').getStore().setData(data);
	controller.since = since;
    },

    layout: 'fit',
    bodyPadding: 15,
    minHeight: 166,

    // we have to wrap the grid in a panel to get the padding right
    items: [
	{
	    xtype: 'grid',
	    reference: 'grid',
	    hideHeaders: true,
	    border: false,
	    bodyBorder: false,
	    rowLines: false,
	    viewConfig: {
		stripeRows: false,
		trackOver: false,
	    },
	    scrollable: false,
	    disableSelection: true,

	    store: {
		data: [],
	    },

	    listeners: {
		cellclick: 'openTaskList',
	    },

	    columns: [
		{
		    dataIndex: 'type',
		    flex: 1,
		},
		{
		    dataIndex: 'error',
		    renderer: 'render_count',
		},
		{
		    dataIndex: 'warning',
		    renderer: 'render_count',
		},
		{
		    dataIndex: 'ok',
		    renderer: 'render_count',
		},
	    ],
	},
    ],

});
