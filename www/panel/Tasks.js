Ext.define('PBS.node.Tasks', {
    extend: 'Ext.grid.GridPanel',

    alias: 'widget.pbsNodeTasks',

    stateful: true,
    stateId: 'pbs-grid-node-tasks',

    loadMask: true,
    sortableColumns: false,

    controller: {
	xclass: 'Ext.app.ViewController',

	showTaskLog: function() {
	    let me = this;
	    let selection = me.getView().getSelection();
	    if (selection.length < 1) {
		return;
	    }

	    let rec = selection[0];

	    Ext.create('Proxmox.window.TaskViewer', {
		upid: rec.data.upid,
		endtime: rec.data.endtime,
	    }).show();
	},

	updateLayout: function() {
	    let me = this;
	    // we want to update the scrollbar on every store load
	    // since the total count might be different
	    // the buffered grid plugin does this only on scrolling itself
	    // and even reduces the scrollheight again when scrolling up
	    me.getView().updateLayout();
	},

	sinceChange: function(field, newval) {
	    let me = this;
	    let vm = me.getViewModel();

	    vm.set('since', newval);
	},

	untilChange: function(field, newval, oldval) {
	    let me = this;
	    let vm = me.getViewModel();

	    vm.set('until', newval);
	},

	reload: function() {
	    let me = this;
	    let view = me.getView();
	    view.getStore().load();
	},

	showFilter: function(btn, pressed) {
	    let me = this;
	    let vm = me.getViewModel();
	    vm.set('showFilter', pressed);
	},

	init: function(view) {
	    let me = this;
	    Proxmox.Utils.monStoreErrors(view, view.getStore(), true);
	},
    },


    listeners: {
	itemdblclick: 'showTaskLog',
    },

    viewModel: {
	data: {
	    typefilter: '',
	    statusfilter: '',
	    datastore: '',
	    showFilter: false,
	    since: null,
	    until: null,
	},

	formulas: {
	    filterIcon: (get) => 'fa fa-filter' + (get('showFilter') ? ' info-blue' : ''),
	    extraParams: function(get) {
		let me = this;
		let params = {};
		if (get('typefilter')) {
		    params.typefilter = get('typefilter');
		}
		if (get('statusfilter')) {
		    params.statusfilter = get('statusfilter');
		}
		if (get('datastore')) {
		    params.store = get('datastore');
		}

		if (get('since')) {
		    params.since = get('since').valueOf()/1000;
		}

		if (get('until')) {
		    let until = new Date(get('until').getTime()); // copy object
		    until.setDate(until.getDate() + 1); // end of the day
		    params.until = until.valueOf()/1000;
		}

		me.getView().getStore().load();

		return params;
	    },
	},

	stores: {
	    bufferedstore: {
		type: 'buffered',
		pageSize: 500,
		autoLoad: true,
		remoteFilter: true,
		model: 'proxmox-tasks',
		proxy: {
		    type: 'proxmox',
		    startParam: 'start',
		    limitParam: 'limit',
		    extraParams: '{extraParams}',
		    url: "/api2/json/nodes/localhost/tasks",
		},
		listeners: {
		    prefetch: 'updateLayout',
		},
	    },
	},
    },

    bind: {
	store: '{bufferedstore}',
    },

    dockedItems: [
	{
	    xtype: 'toolbar',
	    items: [
		{
		    xtype: 'proxmoxButton',
		    text: gettext('View'),
		    iconCls: 'fa fa-window-restore',
		    disabled: true,
		    handler: 'showTaskLog',
		},
		{
		    xtype: 'button',
		    text: gettext('Reload'),
		    iconCls: 'fa fa-refresh',
		    handler: 'reload',
		},
		'->',
		{
		    xtype: 'button',
		    enableToggle: true,
		    bind: {
			iconCls: '{filterIcon}',
		    },
		    text: gettext('Filter'),
		    stateful: true,
		    stateId: 'task-showfilter',
		    stateEvents: ['toggle'],
		    applyState: function(state) {
			if (state.pressed !== undefined) {
			    this.setPressed(state.pressed);
			}
		    },
		    getState: function() {
			return {
			    pressed: this.pressed,
			};
		    },
		    listeners: {
			toggle: 'showFilter',
		    },
		},
	    ],
	},
	{
	    xtype: 'toolbar',
	    dock: 'top',
	    layout: {
		type: 'hbox',
		align: 'top',
	    },
	    bind: {
		hidden: '{!showFilter}',
	    },
	    items: [
		{
		    xtype: 'container',
		    padding: 10,
		    layout: {
			type: 'vbox',
			align: 'stretch',
		    },
		    defaults: {
			labelWidth: 80,
		    },
		    // cannot bind the values directly, as it then changes also
		    // on blur, causing wrong reloads of the store
		    items: [
			{
			    xtype: 'datefield',
			    fieldLabel: gettext('Since'),
			    format: 'Y-m-d',
			    bind: {
				maxValue: '{until}',
			    },
			    listeners: {
				change: 'sinceChange',
			    },
			},
			{
			    xtype: 'datefield',
			    fieldLabel: gettext('Until'),
			    format: 'Y-m-d',
			    bind: {
				minValue: '{since}',
			    },
			    listeners: {
				change: 'untilChange',
			    },
			},
		    ],
		},
		{
		    xtype: 'container',
		    padding: 10,
		    layout: {
			type: 'vbox',
			align: 'stretch',
		    },
		    defaults: {
			labelWidth: 80,
		    },
		    items: [
			{
			    xtype: 'pmxTaskTypeSelector',
			    fieldLabel: gettext('Task Type'),
			    emptyText: gettext('All'),
			    bind: {
				value: '{typefilter}',
			    },
			},
			{
			    xtype: 'combobox',
			    fieldLabel: gettext('Task Result'),
			    emptyText: gettext('All'),
			    multiSelect: true,
			    store: [
				['ok', gettext('OK')],
				['unknown', Proxmox.Utils.unknownText],
				['warning', gettext('Warnings')],
				['error', gettext('Errors')],
			    ],
			    bind: {
				value: '{statusfilter}',
			    },
			},
		    ],
		},
		{
		    xtype: 'container',
		    padding: 10,
		    layout: {
			type: 'vbox',
			align: 'stretch',
		    },
		    defaults: {
			labelWidth: 80,
		    },
		    items: [
			{
			    xtype: 'pbsDataStoreSelector',
			    fieldLabel: gettext('Datastore'),
			    emptyText: gettext('All'),
			    bind: {
				value: '{datastore}',
			    },
			    allowBlank: true,
			},
		    ],
		},
	    ],
	},
    ],

    viewConfig: {
	trackOver: false,
	stripeRows: false, // does not work with getRowClass()
	emptyText: gettext('No Tasks found'),

	getRowClass: function(record, index) {
	    let status = record.get('status');

	    if (status) {
		let parsed = Proxmox.Utils.parse_task_status(status);
		if (parsed === 'error') {
		    return "proxmox-invalid-row";
		} else if (parsed === 'warning') {
		    return "proxmox-warning-row";
		}
	    }
	    return '';
	},
    },

    columns: [
	{
	    header: gettext("Start Time"),
	    dataIndex: 'starttime',
	    width: 130,
	    renderer: function(value) {
		return Ext.Date.format(value, "M d H:i:s");
	    },
	},
	{
	    header: gettext("End Time"),
	    dataIndex: 'endtime',
	    width: 130,
	    renderer: function(value, metaData, record) {
		if (!value) {
		    metaData.tdCls = "x-grid-row-loading";
		    return '';
		}
		return Ext.Date.format(value, "M d H:i:s");
	    },
	},
	{
	    header: gettext("Duration"),
	    hidden: true,
	    width: 80,
	    renderer: function(value, metaData, record) {
		let start = record.data.starttime;
		if (start) {
		    let end = record.data.endtime || Date.now();
		    let duration = end - start;
		    if (duration > 0) {
			duration /= 1000;
		    }
		    return Proxmox.Utils.format_duration_human(duration);
		}
		return Proxmox.Utils.unknownText;
	    },
	},
	{
	    header: gettext("User name"),
	    dataIndex: 'user',
	    width: 150,
	},
	{
	    header: gettext("Description"),
	    dataIndex: 'upid',
	    flex: 1,
	    renderer: Proxmox.Utils.render_upid,
	},
	{
	    header: gettext("Status"),
	    dataIndex: 'status',
	    width: 200,
	    renderer: function(value, metaData, record) {
		if (value === undefined && !record.data.endtime) {
		    metaData.tdCls = "x-grid-row-loading";
		    return '';
		}

		return Proxmox.Utils.format_task_status(value);
	    },
	},
    ],
});
