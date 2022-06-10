Ext.define('PBS.Schema', {

    singleton: true,

    metricServer: {
	'influxdb-http': {
	    type: 'InfluxDB (HTTP)',
	    xtype: 'InfluxDbHttp',
	},
	'influxdb-udp': {
	    type: 'InfluxDB (UDP)',
	    xtype: 'InfluxDbUdp',
	},
    },
});
