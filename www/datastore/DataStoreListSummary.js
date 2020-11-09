// Summary Panel for a single datastore in overview
Ext.define('PBS.datastore.DataStoreListSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreListSummary',
    mixins: ['Proxmox.Mixin.CBind'],

    cbind: {
	title: '{datastore}',
    },
    bodyPadding: 10,

    viewModel: {
	data: {
	    usage: "N/A",
	    full: "N/A",
	    history: [],
	},

	stores: {
	    historystore: {
		data: [],
	    },
	},
    },
    setTasks: function(taskdata, since) {
	let me = this;
	me.down('pbsTaskSummary').updateTasks(taskdata, since);
    },

    setStatus: function(statusData) {
	let me = this;
	let vm = me.getViewModel();
	vm.set('usagetext', PBS.Utils.render_size_usage(statusData.used, statusData.total));
	vm.set('usage', statusData.used/statusData.total);
	let estimate = PBS.Utils.render_estimate(statusData['estimated-full-date']);
	vm.set('full', estimate);
	let last = 0;
	let data = statusData.history.map((val) => {
	    if (val === null) {
		val = last;
	    } else {
		last = val;
	    }
	    return val;
	});
	let historyStore = vm.getStore('historystore');
	historyStore.setData([
	    {
		history: data,
	    },
	]);
    },

    items: [
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },

	    defaults: {
		flex: 1,
		padding: 5,
	    },

	    items: [
		{
		    xtype: 'pmxInfoWidget',
		    iconCls: 'fa fa-fw fa-hdd-o',
		    title: gettext('Usage'),
		    bind: {
			data: {
			    usage: '{usage}',
			    text: '{usagetext}',
			},
		    },
		},
		{
		    xtype: 'pmxInfoWidget',
		    iconCls: 'fa fa-fw fa-line-chart',
		    title: gettext('Estimated Full'),
		    printBar: false,
		    bind: {
			data: {
			    usage: '0',
			    text: '{full}',
			},
		    },
		},
	    ],
	},
	{
	    // we cannot autosize a sparklineline widget,
	    // abuse a grid with a single column/row to do it for us
	    xtype: 'grid',
	    hideHeaders: true,
	    minHeight: 50,
	    border: false,
	    bodyBorder: false,
	    rowLines: false,
	    disableSelection: true,
	    viewConfig: {
		trackOver: false,
	    },
	    bind: {
		store: '{historystore}',
	    },
	    columns: [{
		xtype: 'widgetcolumn',
		flex: 1,
		dataIndex: 'history',
		widget: {
		    xtype: 'sparklineline',
		    bind: '{record.history}',
		    spotRadius: 0,
		    fillColor: '#ddd',
		    lineColor: '#555',
		    lineWidth: 0,
		    chartRangeMin: 0,
		    chartRangeMax: 1,
		    tipTpl: '{y:number("0.00")*100}%',
		    height: 40,
		},
	    }],
	},
	{
	    xtype: 'pbsTaskSummary',
	    border: false,
	    header: false,
	    subPanelModal: true,
	    bodyPadding: 0,
	    minHeight: 0,
	    cbind: {
		datastore: '{datastore}',
	    },
	},
    ],
});
